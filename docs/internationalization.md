# Internationalization (i18n)

Code Intel Pipeline now supports multiple languages for reports and output.

## Supported Languages

- **English** (default)
- **Chinese Simplified** (中文)

## Configuration

Set language via environment variable:

```bash
export CODE_INTEL_LANG=zh
```

Or:
```bash
export CODE_INTEL_LANG=c
```

Default is English if not set.

## Usage

### Model Routing with Chinese Messages

```bash
CODE_INTEL_LANG=zh code-intel model route --request inventory.json --out result.json
```

### Doctor Bootstrap

```bash
CODE_INTEL_LANG=zh code-intel doctor --repo /path/to/repo
```

## Messages Translated

- Status indicators: `ready`, `consent_required`, `deterministic_degraded`
- Error categories: provider unavailable, model unavailable, config errors
- Cost scopes: local compute, metered API, subscription
- Credentials and authentication state

## Adding More Languages

Extend `src/i18n.rs`:

```rust
pub enum Language {
    English,
    Chinese,
    Spanish,  // Add here
}

pub fn message(&self) -> &'static str {
    match self.lang {
        Language::English => "...",
        Language::Chinese => "...",
        Language::Spanish => "...",  // Add translation
    }
}
```

Update env parsing:
```rust
Some('s') => Language::Spanish,
```

## Output Examples

### English (default)

```json
{
  "status": "ready",
  "failureCategory": "provider_unavailable",
  "reason": "Model unavailable"
}
```

### Chinese (`CODE_INTEL_LANG=zh`)

```json
{
  "status": "已就绪",
  "failureCategory": "提供者不可用",
  "reason": "模型不可用"
}
```

## Scope

Currently applied to:
- Model channel routing messages
- Error and status descriptions
- CLI output labels

Not yet translated:
- Repowise web dashboard (separate project)
- Repository content analysis
- Code-specific identifiers
