# CC Switch Integration

Code Intel Pipeline now supports third-party LLM provider discovery via **CC Switch** configuration broker.

## How It Works

When routing model channels, the pipeline queries CC Switch for available providers. If CC Switch is configured, its candidates are merged into the inventory before routing policy evaluation.

## Setup

### 1. Enable CC Switch Endpoint

Set environment variable:
```bash
export CODE_INTEL_CC_SWITCH_ENDPOINT=https://cc-switch.example.com
```

### 2. Optional: API Authentication

If CC Switch requires authentication:
```bash
export CODE_INTEL_CC_SWITCH_API_KEY=your-api-key
```

**HTTPS is required whenever an API key is set.** An API key is a credential;
sending it as a Bearer token over plain HTTP would put it on the wire in
cleartext. If `CODE_INTEL_CC_SWITCH_API_KEY` is set but
`CODE_INTEL_CC_SWITCH_ENDPOINT` does not start with `https://`, routing fails
fast with a clear error *before any request is sent* -- this is not a network
failure, it's a refusal to make the request at all. An `http://` endpoint with
no API key configured (e.g. local dev) is still allowed, since there's no
secret in flight.

### 3. CC Switch API Contract

CC Switch must expose:

```text
GET /api/channels
```

**Response format:**
```json
{
  "channels": [
    {
      "id": "custom-gpt4",
      "channelKind": "local_compatible",
      "provider": "openai",
      "model": "gpt-4",
      "costScope": "metered_api",
      "endpointConfigured": true,
      "discovered": true,
      "executableVerified": true,
      "authPresent": "present",
      "modelAvailable": "available",
      "externalEgress": true
    }
  ]
}
```

## Integration Flow

```text
code-intel model route
  ↓
load inventory (user-provided candidates)
  ↓
[CODE_INTEL_CC_SWITCH_ENDPOINT set?]
  ├─ YES → query CC Switch /api/channels
  │         merge candidates into inventory
  ├─ NO  → skip CC Switch
  ↓
validate inventory
  ↓
evaluate routing policy (existing logic applies)
  ↓
return selected channel
```

## Example

### 1. Configure CC Switch

```bash
export CODE_INTEL_CC_SWITCH_ENDPOINT=http://localhost:3000
```

### 2. Route model with CC Switch candidates

```bash
code-intel model route --request inventory.json --out result.json
```

The tool will:
- Detect `CODE_INTEL_CC_SWITCH_ENDPOINT`
- Call `http://localhost:3000/api/channels`
- Merge results with `inventory.json` candidates
- Apply existing routing policy

### 3. Result

```json
{
  "status": "ready",
  "selected": {
    "candidateId": "custom-gpt4",
    "provider": "openai",
    "model": "gpt-4"
  }
}
```

## Error Handling

CC Switch failures do **not** fall back to user candidates. Any failure while
querying CC Switch -- unreachable endpoint, non-200 response, unparseable
response, missing `channels` array, or the HTTPS/API-key guard above -- makes
`merge_cc_switch_candidates` return an error, which propagates out of `route`.
`run_raw` then exits with code `65` and prints the error to stderr; no result
JSON is written. The upstream HTTP status code is included in that stderr
message (e.g. `CC Switch returned status 503`) but is not a structured field
in any returned JSON -- there is nothing for a caller to parse it out of.

- **CC Switch unreachable / non-200 / bad response**: `route` returns `Err`, `run_raw` exits `65`
- **`CODE_INTEL_CC_SWITCH_API_KEY` set with a non-`https://` endpoint**: rejected before any request is sent, `run_raw` exits `65`
- **`CODE_INTEL_CC_SWITCH_ENDPOINT` unset**: CC Switch is skipped entirely; routing proceeds with only the user-provided candidates

## Security Notes

- CC Switch credentials never stored in artifacts
- Only boolean presence signals used for authorization decisions
- API calls timeout after 5 seconds to prevent hangs
- Bearer token auth via `CODE_INTEL_CC_SWITCH_API_KEY`
