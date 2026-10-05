#!/usr/bin/env python3
"""Replay frozen pre-migration requests against the actual compiled CLI.

--cli EXE --capture CAPTURE_JSON --output NEW_DIRECTORY
Standard library only; never builds or installs Rust. Every old expected output
and payload comes from the recorded corpus, not the current implementation.
"""
import http.client
import os
import socket
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from http_capture_format import blob, compare_approved, dump, emit, load_blob, load_json, materialize_capture, normalized, parse_args, sha256_file

SCHEMA = 'desk417-real-cli-http.v1'
GROUPS = [f'M{i}' for i in range(1, 7)] + [f'P{i}' for i in range(1, 7)] + ['R1', 'R2']




class ControlledServer(ThreadingHTTPServer):
    daemon_threads = True
    block_on_close = False


def start_server(case, source, output, git_fixture):
    events = []
    lock = threading.Lock()
    class Handler(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'
        def version_string(self):
            # Freeze our controlled peer's observed banner across replay hosts.
            return 'BaseHTTP/0.6 Python/3.14.7'
        def handle(self):
            try:
                super().handle()
            except (ConnectionResetError, ConnectionAbortedError):
                # The finite model process can close before reading error bodies.
                pass
        def log_message(self, *args):
            pass
        def handle_any(self):
            if self.headers.get('Transfer-Encoding', '').lower() == 'chunked':
                chunks = []
                while True:
                    length = int(self.rfile.readline().split(b';', 1)[0].strip(), 16)
                    if length == 0:
                        while True:
                            trailer = self.rfile.readline()
                            if trailer == b'\r\n':
                                break
                            if not trailer:
                                raise ValueError('Truncated controlled request chunk trailer')
                        break
                    chunk = self.rfile.read(length)
                    if len(chunk) != length or self.rfile.read(2) != b'\r\n':
                        raise ValueError('Invalid controlled request chunk framing')
                    chunks.append(chunk)
                body = b''.join(chunks)
            else:
                body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
            event = {'method': self.command, 'path': self.path, 'headers': list(self.headers.raw_items()), 'body': blob(output, body)}
            with lock:
                events.append(event)
            if self.path == '/api/repos':
                spec = case['repos']
                response = load_blob(source, spec['body']).replace(b'<GIT_FIXTURE>', str(git_fixture).replace('\\', '/').encode())
                status = spec['status']
                headers = [['Content-Type', 'application/json']]
                delay = 0
            elif self.path.startswith('/redirect/'):
                hops = int(self.path.rsplit('/', 1)[1])
                if hops > 0:
                    status, response = 302, b'redirect'
                    headers = [['Location', f'http://localhost:{self.server.server_port}/redirect/{hops - 1}'], ['Content-Type', 'text/plain']]
                else:
                    status, response, headers = 200, b'redirect-complete', [['Content-Type', 'text/plain']]
                delay = 0
            elif case.get('redirect_status'):
                if self.path.split('?', 1)[0] == '/redirect-target':
                    status, response, headers = 200, b'redirect-complete', [['Content-Type', 'text/plain']]
                else:
                    status, response, headers = case['redirect_status'], b'redirect', [['Location', case['location']], ['Content-Type', 'text/plain']]
                delay = 0
            else:
                spec = case['upstream']
                status, response, headers = spec['status'], load_blob(source, spec['body']), spec['headers']
                delay = case.get('body_delay', 0)
            try:
                self.send_response(status)
                for name, value in headers:
                    self.send_header(name, value)
                self.send_header('Content-Length', str(len(response)))
                self.end_headers()
                self.wfile.flush()
                if delay:
                    time.sleep(delay)
                self.wfile.write(response)
            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                pass
        def __getattr__(self, name):
            if name.startswith('do_'):
                return self.handle_any
            raise AttributeError(name)
    server = ControlledServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, events


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def wait_until(predicate, seconds, message):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    raise TimeoutError(message)


def client_request(port, spec, source, output):
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=30)
    body = spec['literal_body'] if 'literal_body' in spec else load_blob(source, spec['body']) if 'body' in spec else b''
    try:
        conn.putrequest(spec.get('method', 'GET'), spec.get('path', '/fixture'), skip_accept_encoding=True)
        conn.putheader('Content-Length', str(len(body)))
        for name, value in spec.get('headers', []):
            conn.putheader(name, value)
        conn.endheaders(body)
        response = conn.getresponse()
        return {'status': response.status, 'headers': response.getheaders(), 'body': blob(output, response.read())}
    finally:
        conn.close()




def make_fixture(root):
    repo = root / 'git-fixture'
    repo.mkdir(parents=True, exist_ok=False)
    commands = [['git', 'init', str(repo)], ['git', '-C', str(repo), 'remote', 'add', 'origin', 'https://github.com/fixture-owner/fixture-repo.git']]
    observations = []
    for command in commands:
        result = subprocess.run(command, capture_output=True, text=True)
        observations.append({'command': command, 'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr})
        if result.returncode:
            raise RuntimeError(observations)
    return repo, observations


def execute(case, cli, source, output, session, repo):
    root = session / case['id']
    root.mkdir()
    env = dict(os.environ)
    for key in list(env):
        if key.upper() in ['CODE_INTEL_CC_SWITCH_ENDPOINT', 'CODE_INTEL_CC_SWITCH_API_KEY', 'HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY', 'CODE_INTEL_DATA_ROOT', 'CODE_INTEL_LANG']:
            env.pop(key)
    env.update({'CODE_INTEL_DATA_ROOT': str(root / 'data'), 'CODE_INTEL_LANG': case['language'],
                'HOME': str(root), 'USERPROFILE': str(root), 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1'})
    if case.get('bad_environment_proxy'):
        env.update({name: 'http://127.0.0.1:1' for name in ['HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy']})
        env.update({'NO_PROXY': '', 'no_proxy': ''})
    server, events = start_server(case, source, output, repo)
    upstream_port = server.server_port
    proxy_port = free_port()
    raw = {'case': case['id'], 'group': case['group'], 'requests': events, 'client_response': None,
           'environment': {key: env[key] for key in env if key.startswith('CODE_INTEL_') or key.upper() in ['HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY']}}
    substitutions = [(str(cli), '<CLI>'), (str(root), '<CASE_ROOT>'), (str(repo), '<GIT_FIXTURE>'),
                     (str(source), '<CAPTURE>'), (str(upstream_port), '<UPSTREAM_PORT>'), (str(proxy_port), '<PROXY_PORT>')]
    process = None
    started = time.monotonic()
    try:
        if case['kind'] == 'model':
            fixture = root / 'request.json'
            fixture.write_bytes(load_blob(source, case['fixture']))
            if case.get('endpoint', True):
                env['CODE_INTEL_CC_SWITCH_ENDPOINT'] = f'http://127.0.0.1:{upstream_port}'
            if 'synthetic_key' in case:
                env['CODE_INTEL_CC_SWITCH_API_KEY'] = case['synthetic_key']
            command = [str(cli), 'model', 'route', '--request', str(fixture)]
            result = subprocess.run(command, env=env, cwd=root, capture_output=True, timeout=30)
            raw.update(command=command, exit=result.returncode, stdout=result.stdout.decode('utf-8', 'replace'), stderr=result.stderr.decode('utf-8', 'replace'))
        else:
            if case.get('transport_failure'):
                server.shutdown()
                server.server_close()
            command = [str(cli), 'repowise-proxy', str(upstream_port), str(proxy_port)]
            process = subprocess.Popen(command, env=env, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            def ready():
                if process.poll() is not None:
                    raise RuntimeError('Proxy exited before listen')
                with socket.socket() as s:
                    return s.connect_ex(('127.0.0.1', proxy_port)) == 0
            wait_until(ready, 10, 'Proxy did not listen')
            if not case.get('transport_failure'):
                if case.get('registry_failure'):
                    wait_until(lambda: len([e for e in events if e['path'] == '/api/repos']) >= 2, 10, 'No second registry retry')
                else:
                    wait_until(lambda: (root / 'data/remote-links/registry.json').exists(), 10, 'Registry did not warm')
            if case['kind'] == 'registry':
                client = {'method': 'GET', 'path': '/__code-intel/remote-links.json'}
            else:
                client = case.get('client', {'method': 'GET', 'path': '/fixture'})
                if 'redirects' in case:
                    client = {'method': 'GET', 'path': '/redirect/' + str(case['redirects'])}
            raw['client_response'] = client_request(proxy_port, client, source, output)
            raw['command'] = command
            if process.poll() is not None:
                raise RuntimeError('Proxy exited before harness cleanup')
            process.terminate()
            stdout, stderr = process.communicate(timeout=10)
            raw.update(exit=process.returncode, stdout=stdout.decode('utf-8', 'replace'), stderr=stderr.decode('utf-8', 'replace'), termination='harness terminate after public HTTP capture')
    except Exception as error:
        raw['harness_error'] = repr(error)
        raw.setdefault('command', [str(cli)])
        raw.setdefault('stdout', '')
        raw.setdefault('stderr', '')
        raw.setdefault('exit', None)
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                stdout, stderr = process.communicate(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                stdout, stderr = process.communicate()
            raw.update(exit=process.returncode, stdout=stdout.decode('utf-8', 'replace'), stderr=stderr.decode('utf-8', 'replace'))
        server.shutdown()
        server.server_close()
    raw['elapsed_seconds_diagnostic'] = round(time.monotonic() - started, 3)
    raw['environment'] = {key: env[key] for key in env if key.startswith('CODE_INTEL_') or key.upper() in ['HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY']}
    raw['requests'] = list(events)
    raw['normalized'] = normalized(raw, substitutions)
    dump(output / (case['id'] + '.raw.json'), raw)
    return raw


def main():
    args = parse_args(__doc__)
    cli = args.cli.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True)
    source = materialize_capture(args.capture.resolve(), output / 'source')
    cases = [load_json(p) for p in sorted(source.glob('*.request.json'))]
    if args.only:
        selected = args.only.split(',')
        cases = [c for c in cases if c['id'] in selected or c['group'] in selected]
    session = output / 'runtime'
    session.mkdir()
    repo, git_setup = make_fixture(session)
    summary = {'schema': SCHEMA, 'mode': 'replay', 'cli': str(cli), 'cli_sha256': sha256_file(cli),
               'source': str(args.capture.resolve()), 'output': str(output), 'groups': [], 'cases': [], 'git_fixture_setup': git_setup,
               'normalization': ['Temporary paths and ports', 'HTTP Date', 'Header-name case and ordering across different names; retain value order within name',
                                 'Library User-Agent version and library-generated error suffix diagnostic only; application prefix/category/status retained'],
               'explicit_gaps': ['Loopback HTTP only; real HTTPS credential sending, DNS/connect timeout and TLS behavior not exercised',
                                 'R2 records first two genuine retries and public empty result; eight-attempt exhaustion/backoff end not waited',
                                 'Proxy is intentionally terminated after HTTP capture; its exit is process-cleanup exit, not a finite product command result']}
    failed = False
    for case in cases:
        raw = execute(case, cli, source, output, session, repo)
        actual = raw['normalized']
        dump(output / (case['id'] + '.actual.json'), actual)
        approved = load_json(source / (case['id'] + '.approved.json'))
        matched, expected, observed, notes = compare_approved(actual, approved, case, source, output)
        dump(output / (case['id'] + '.comparison.json'), {
            'matches_approved': matched, 'normalization': notes,
            'expected': expected, 'actual': observed,
        })
        if not matched:
            failed = True
            emit({'id': case['id'], 'expected': expected, 'actual': observed, 'normalization': notes})
        error = raw.get('harness_error')
        failed = failed or bool(error)
        entry = {'id': case['id'], 'group': case['group'], 'command': actual['command'], 'exit': raw['exit'], 'stderr': actual['stderr'],
                 'stdout': actual['stdout'], 'response': actual['client_response'], 'forwarded': actual['requests'],
                 'elapsed_seconds_diagnostic': raw['elapsed_seconds_diagnostic'], 'harness_error': error, 'matches_approved': matched}
        summary['cases'].append(entry)
        emit({'id': case['id'], 'exit': raw['exit'], 'http': raw.get('client_response', {}).get('status') if raw.get('client_response') else None,
              'requests': len(raw['requests']), 'harness_error': error, 'matches_approved': matched})
    # New boundary requirement: independent literal outcome, not a new approved mother copy.
    if not args.only or 'P6' in args.only.split(','):
        case = load_json(source / 'P6-redirect-5.request.json')
        case.update(id='N1-four-redirects', group='N1', redirects=4)
        raw = execute(case, cli, source, output, session, repo)
        response = raw.get('client_response')
        paths = [event['path'] for event in raw['requests'] if event['path'].startswith('/redirect/')]
        matched = bool(response and response['status'] == 200 and not raw.get('harness_error')
                       and load_blob(output, response['body']) == b'redirect-complete'
                       and paths == ['/redirect/4', '/redirect/3', '/redirect/2', '/redirect/1', '/redirect/0'])
        summary['new_requirements'] = [{'id': case['id'], 'matches_expected': matched,
                                        'expected_status': 200, 'expected_body': 'redirect-complete',
                                        'actual': raw['normalized']}]
        failed = failed or not matched
        emit({'id': case['id'], 'matches_expected': matched})
        # Literal old public contracts: refuse unsafe replay, but retain allowed redirects.
        for identifier, method, status, location, expected_status, expected_body, expected_location, expected_requests in [
            ('N2-post-307', 'POST', 307, '/redirect-target', 307, b'redirect', '/redirect-target', [('POST', '/fixture', b'abc')]),
            ('N3-post-308', 'POST', 308, '/redirect-target', 308, b'redirect', '/redirect-target', [('POST', '/fixture', b'abc')]),
            ('N4-post-302', 'POST', 302, 'next/../redirect-target?fixture=ok#ignored', 200, b'redirect-complete', None, [('POST', '/fixture', b'abc'), ('GET', '/redirect-target?fixture=ok', b'')]),
            ('N5-options-307', 'OPTIONS', 307, '/redirect-target', 200, b'redirect-complete', None, [('OPTIONS', '/fixture', b'abc'), ('OPTIONS', '/redirect-target', b'')]),
        ]:
            case = load_json(source / 'P2-empty-post.request.json')
            case.update(id=identifier, group=identifier, redirect_status=status, location=location,
                        client={'method': method, 'path': '/fixture', 'literal_body': b'abc'})
            raw = execute(case, cli, source, output, session, repo)
            response = raw.get('client_response')
            requests = [(event['method'], event['path'], load_blob(output, event['body']))
                        for event in raw['requests'] if event['path'] != '/api/repos']
            actual_location = next((value for name, value in response['headers'] if name.lower() == 'location'), None) if response else None
            matched = bool(response and not raw.get('harness_error') and response['status'] == expected_status
                           and load_blob(output, response['body']) == expected_body
                           and actual_location == expected_location and requests == expected_requests)
            summary['new_requirements'].append({'id': identifier, 'matches_expected': matched,
                                                'expected_status': expected_status, 'expected_body': expected_body.decode(),
                                                'actual': raw['normalized']})
            failed = failed or not matched
            emit({'id': identifier, 'matches_expected': matched, 'actual_status': response['status'] if response else None})
    summary['groups'] = sorted(set(c['group'] for c in cases))
    summary['all_14_groups_exercised'] = set(summary['groups']) == set(GROUPS)
    summary['harness_errors'] = [c['id'] for c in summary['cases'] if c['harness_error']]
    summary['exit'] = int(failed)
    dump(output / 'summary.json', summary)
    emit({'groups': summary['groups'], 'cases': len(cases), 'harness_errors': summary['harness_errors'], 'exit': int(failed)})
    return int(failed)


if __name__ == '__main__':
    raise SystemExit(main())
