# Python 3.14 development lane with optional Pon parity

## Goal

Make Python 3.14 an executable development agreement for Code Intel Pipeline while treating Pon as
an optional native backend whose compatibility must be proven, never assumed.

## Contract

1. CPython 3.14 is the authoritative runtime and semantic oracle.
2. Repository-owned Python entry points must compile under CPython 3.14.
3. A reviewed corpus pins portable expected behavior under CPython 3.14.
4. When Pon is available, the Pon-required corpus is dual-run and stdout, stderr, and exit code must
   exactly match CPython on the same machine.
5. A development profile may pass with Pon unavailable, but must report that state explicitly.
6. A Pon-candidate profile fails unless Pon is present and every required dual-run case matches.
7. Python 3.14 features not yet claimed for Pon remain explicit CPython-only cases rather than being
   removed from the project language agreement.

## Boundaries

- Do not install, vendor, or execute upstream Pon source automatically.
- Do not change the user's dirty CI workflow in this pass.
- Do not change the system/default `python`; resolve Python 3.14 explicitly.
- Do not describe CPython-only success as Pon compatibility.
- Reuse the project conformance runner by adding this lane as an executable suite.

## Verification

- Development profile passes on CPython 3.14 with an explicit `pon=unavailable` observation.
- Pon-candidate fails closed when Pon is absent.
- A test-only CPython-backed Pon shim proves the positive dual-run path.
- A divergent shim is rejected by the exact parity gate.
- The project fast conformance profile executes the Python 3.14 development suite.
