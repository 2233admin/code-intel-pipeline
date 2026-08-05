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

The following surface as additive `*Display` fields alongside their canonical,
never-translated counterparts (see Output Examples below):

- Status indicators: `ready`, `consent_required`, `deterministic_degraded`
- Error categories: provider unavailable, model unavailable, config errors
- Cost scopes: local compute, metered API, subscription
- Credentials and authentication state

## Adding More Languages

Extend `crates/code-intel-cli/src/i18n.rs`:

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

`status`, `readinessState`, `failureCategory`, and `reason` are canonical v1
protocol fields (`code-intel-model-routing-result.v1`) that `run_raw` and
external callers match on verbatim -- they are **never** translated,
regardless of `CODE_INTEL_LANG`. Localization instead lands in additive
`statusDisplay` / `readinessStateDisplay` / `reasonDisplay` companion fields,
so existing consumers reading the canonical fields are unaffected. In English
(the default), every `*Display` field is simply equal to its canonical
counterpart; only under `CODE_INTEL_LANG=zh` do the `*Display` fields diverge,
and only for tokens that have a Chinese mapping (a token with no mapping falls
back to the canonical, untranslated value even under `zh` -- see `reason` /
`reasonDisplay` below, where `model_not_available` has no Chinese mapping yet).

### English (default)

```json
{
  "schema": "code-intel-model-routing-result.v1",
  "status": "consent_required",
  "statusDisplay": "consent_required",
  "attempts": [
    {
      "candidateId": "local-llama",
      "readinessState": "model_available",
      "readinessStateDisplay": "model_available",
      "eligible": false,
      "failureCategory": "model_unavailable",
      "reason": "model_not_available",
      "reasonDisplay": "model_not_available"
    }
  ]
}
```

### Chinese (`CODE_INTEL_LANG=zh`)

```json
{
  "schema": "code-intel-model-routing-result.v1",
  "status": "consent_required",
  "statusDisplay": "需要确认",
  "attempts": [
    {
      "candidateId": "local-llama",
      "readinessState": "model_available",
      "readinessStateDisplay": "模型可用",
      "eligible": false,
      "failureCategory": "model_unavailable",
      "reason": "model_not_available",
      "reasonDisplay": "model_not_available"
    }
  ]
}
```

Note that `status`, `readinessState`, `failureCategory`, and `reason` are
byte-identical between the two examples above -- only the `*Display` fields
changed.

## Scope

Currently applied to:
- Model channel routing messages
- Error and status descriptions
- CLI output labels

Not yet translated:
- Repowise web dashboard (separate project)
- Repository content analysis
- Code-specific identifiers
