"""Frozen pre-migration HTTP fixture bytes and explicitly approved comparison rules."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import zlib


def materialize_capture(path, root):
    if path.is_dir():
        return path
    corpus = load_json(path)
    if corpus['schema'] != 'desk417-http-mother-copy.v1':
        raise ValueError('Unknown HTTP mother-copy schema')
    (root / 'blobs').mkdir(parents=True)
    for digest, entry in corpus['blobs'].items():
        data = zlib.decompress(base64.b64decode(entry['zlibBase64'], validate=True))
        if hashlib.sha256(data).hexdigest() != digest or len(data) != entry['length']:
            raise ValueError('Frozen HTTP payload digest/length mismatch: ' + digest)
        (root / 'blobs' / (digest + '.bin')).write_bytes(data)
    for section, suffix in [('requests', '.request.json'), ('approved', '.approved.json')]:
        for key, value in corpus[section].items():
            (root / (key + suffix)).write_text(json.dumps(value, ensure_ascii=False) + '\n', encoding='utf-8')
    return root


def html_dictionary_fingerprint(root, ref):
    data = (root / 'blobs' / (ref['sha256'] + '.bin')).read_bytes()
    if len(data) != ref['length'] or hashlib.sha256(data).hexdigest() != ref['sha256']:
        raise ValueError('Response body no longer matches its frozen/raw fingerprint')
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        return ref
    if 'id="repowise-i18n-injected"' not in text:
        return ref
    matches = list(re.finditer(r'var DICT = new Map\((\[.*?\])\);', text))
    if len(matches) != 1:
        return ref
    match = matches[0]
    pairs = json.loads(match.group(1))
    if not isinstance(pairs, list) or not all(isinstance(pair, list) and len(pair) == 2 and all(isinstance(value, str) for value in pair) for pair in pairs):
        return ref
    if len({pair[0] for pair in pairs}) != len(pairs):
        return ref
    # Keep every key/value and every byte outside this one order-insensitive Map.
    ordered = json.dumps(sorted(pairs), ensure_ascii=False, separators=(',', ':'))
    canonical = (text[:match.start(1)] + ordered + text[match.end(1):]).encode('utf-8')
    return {'length': len(canonical), 'sha256': hashlib.sha256(canonical).hexdigest()}


def temporary_paths(value):
    return re.sub(r'<(?:CASE_ROOT|GIT_FIXTURE|CAPTURE)>[^\s"]*',
                  lambda match: match.group().replace('\\', '/'), value)


def compare_approved(actual, approved, case, source, output):
    # Caller persists raw evidence first; these newly decoded comparison objects are owned here.
    expected = approved
    observed = actual
    if case['kind'] != 'model' and observed['harness_error'] is None:
        # Recorded proxy exits are harness cleanup, not a finite CLI result.
        # The runner rejects a proxy that exits before the intentional termination.
        expected['exit'] = observed['exit'] = '<HARNESS_TERMINATION>'
    for document in (expected, observed):
        document['command'] = [temporary_paths(argument) for argument in document['command']]
        for field in ('stdout', 'stderr'):
            document[field] = temporary_paths(document[field])
    notes = []
    if case['id'] == 'P2-empty-post':
        empty_digest = hashlib.sha256(b'').hexdigest()
        for request in observed['requests']:
            if request['method'] == 'POST' and request['body'] == {'length': 0, 'sha256': empty_digest}:
                headers = [header for header in request['headers'] if header != ['transfer-encoding', ['chunked']]]
                if headers != request['headers']:
                    request['headers'] = headers
                    notes.append('User-approved zero-body POST chunked framing; all other headers retained')
    if case['id'] == 'P4-html-zh':
        for document, root in [(expected, source), (observed, output)]:
            response = document['client_response']
            response['body'] = html_dictionary_fingerprint(root, response['body'])
        notes.append('Unique DICT entry order only; key/value pairs and all other body bytes retained')
    return observed == expected, expected, observed, notes


def dump(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def blob(root, data):
    digest = hashlib.sha256(data).hexdigest()
    name = f'blobs/{digest}.bin'
    path = root / name
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    result = {'file': name, 'length': len(data), 'sha256': digest}
    if len(data) <= 8192:
        try:
            result['text'] = data.decode('utf-8')
        except UnicodeDecodeError:
            result['base64'] = base64.b64encode(data).decode()
    return result


def load_blob(root, ref):
    data = (root / ref['file']).read_bytes()
    if len(data) != ref['length'] or hashlib.sha256(data).hexdigest() != ref['sha256']:
        raise ValueError(f'Input blob changed: {ref}')
    return data
def scrub_text(value, substitutions):
    for old, new in sorted(substitutions, key=lambda pair: -len(pair[0])):
        value = value.replace(old, new).replace(old.replace('\\', '/'), new)
    return value


def app_error(text):
    """Library-generated suffix stays in raw diagnostics, category remains contractual."""
    if text.startswith('CC Switch request failed:'):
        status = re.search(r'(?:status code|http status:?|StatusCode\()\s*(\d{3})', text, re.I)
        if status:
            return 'CC Switch request failed: HTTP ' + status.group(1)
        category = 'timeout' if re.search('timed out|timeout|os error 10060', text, re.I) else 'transport'
        return 'CC Switch request failed: ' + category
    if text.startswith('CC Switch response parse failed:'):
        category = 'timeout' if re.search('timed out|timeout|os error 10060', text, re.I) else 'invalid JSON'
        return 'CC Switch response parse failed: ' + category
    if text.startswith('Upstream error for '):
        prefix = text.split(': ', 1)[0]
        category = 'redirect limit' if re.search('redirect', text, re.I) else 'transport'
        return prefix + ': ' + category
    return text


def normalized(raw, substitutions):
    def headers(items):
        grouped = {}
        for name, value in items:
            name = name.lower()
            if name == 'user-agent':
                value = '<LIBRARY-USER-AGENT-DIAGNOSTIC>'
            elif name == 'date':
                value = '<HTTP-DATE>'
            grouped.setdefault(name, []).append(scrub_text(value, substitutions))
        return [[name, values] for name, values in sorted(grouped.items())]
    def body(ref):
        # Full bytes remain in blob files; hash+length are exact byte comparison.
        return {key: ref[key] for key in ['length', 'sha256']}
    def event(item):
        result = {key: item[key] for key in ['method', 'path'] if key in item}
        if 'status' in item:
            result['status'] = item['status']
        result['headers'] = headers(item['headers'])
        result['body'] = body(item['body'])
        return result
    return {'command': [scrub_text(x, substitutions) for x in raw['command']],
            'exit': raw['exit'], 'stdout': scrub_text(raw['stdout'], substitutions),
            'stderr': '\n'.join(app_error(line) for line in scrub_text(raw['stderr'], substitutions).splitlines()),
            'requests': [event(x) for x in raw['requests']],
            'client_response': event(raw['client_response']) if raw.get('client_response') else None,
            'harness_error': raw.get('harness_error')}


def parse_args(description):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--only', help='Comma-separated recorded case IDs or groups (optional focused replay)')
    args = parser.parse_args()
    if args.output.exists():
        parser.error('replay refuses existing output directory; never overwrites evidence')
    return args


def load_json(path):
    return json.loads(path.read_text(encoding='utf-8'))


def sha256_file(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def emit(value):
    print(json.dumps(value), flush=True)
