# Python 3.14 development and optional Pon compatibility

Code Intel Pipeline uses CPython 3.14 as the authoritative Python language/runtime agreement. Pon
is an optional native execution backend. A project can therefore use Python 3.14 without claiming
that Pon supports every Python 3.14 feature.

## Profiles

- `development`: CPython 3.14 must exist, repository Python entry points must compile, and the full
  local corpus must match reviewed golden behavior. Pon is dual-run when available; absence is
  reported but does not block ordinary development.
- `pon-candidate`: all development checks pass, Pon must exist, and every `ponRequired` case must
  exactly match CPython stdout, stderr, and exit code on the same machine.

Run:

```powershell
./scripts/tests/Test-Python314PonCompatibility.ps1 -Profile development
./scripts/tests/Test-Python314PonCompatibility.ps1 -Profile pon-candidate -PonCommand pon -Json
```

The policy is `orchestration/python314-pon-development-policy.v1.json`. The corpus manifest is
`tests/fixtures/python314-compat/manifest.v1.json`.

The corpus deliberately separates two claims:

- Core cases marked `ponRequired: true` define the current candidate portability subset.
- The Python 3.14 template-string case is CPython-authoritative but not yet required from Pon. This
  keeps Python 3.14 language development open without overstating the alternative backend.

## Team agreement

1. Use explicit Python 3.14 selection; do not rely on whichever `python` happens to be first on PATH.
2. CPython behavior wins when runtimes disagree.
3. A Pon difference is a failed compatibility observation, not permission to weaken the CPython
   golden or silently exclude a case.
4. Promote a Python feature into `ponRequired` only after a real Pon run passes and the change is
   reviewed.
5. The gate never installs or executes upstream source automatically. A Pon executable must be
   supplied by the environment/operator.
