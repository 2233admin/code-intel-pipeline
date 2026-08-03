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

### 3. CC Switch API Contract

CC Switch must expose:

```
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

```
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

- **CC Switch unreachable**: routing continues with user candidates; error logged
- **Invalid response**: inventory validation fails with detailed error
- **HTTP errors**: returned to caller with CC Switch status code

## Security Notes

- CC Switch credentials never stored in artifacts
- Only boolean presence signals used for authorization decisions
- API calls timeout after 5 seconds to prevent hangs
- Bearer token auth via `CODE_INTEL_CC_SWITCH_API_KEY`
