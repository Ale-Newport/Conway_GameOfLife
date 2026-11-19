# Security policy

## Supported versions

Emergence Lab is pre-1.0. Security fixes go into the latest release and the
`main` branch.

| Version | Supported |
|---|---|
| latest release (`0.1.x`) | ✅ |
| `main` | ✅ |
| older | ❌ |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report them privately through GitHub's
[private vulnerability reporting](https://github.com/Ale-Newport/Conway_GameOfLife/security/advisories/new)
(*Security* tab → *Report a vulnerability*). Include:

* what is affected (crate, file format, CLI command, web demo, workflow);
* how to reproduce it, ideally with a minimal file or scenario;
* the impact you expect.

You can expect an acknowledgement within 7 days and a status update within
30 days. Once a fix is released the advisory is published with credit to
you, unless you prefer to stay anonymous.

## Scope

Emergence Lab is an offline desktop application, a command-line tool and a
static web demo. It has no server, accounts or network services. Relevant
issues include, for example:

* crashes, hangs or unbounded memory use caused by crafted **input files**:
  RLE / Life 1.06 / macrocell patterns, RON scenarios, `.elsnap` snapshots,
  custom-life DSL or Rhai scripts;
* escapes from the **Rhai sandbox** used by the Custom Life Creator (scripts
  must not reach the file system, the network or unbounded CPU/memory);
* path traversal when saving or loading user files;
* vulnerable **dependencies** (also tracked by Dependabot and `cargo-deny`);
* weaknesses in the **GitHub Actions** workflows of this repository.

## How the project is protected

* Every pull request needs the maintainer's approval (`.github/CODEOWNERS`
  and repository rulesets). Force pushes and branch deletion are blocked for
  everyone else.
* CI runs `cargo-deny` (RustSec advisories, licences, sources), CodeQL,
  gitleaks secret scanning and dependency review. GitHub secret scanning with
  push protection and Dependabot alerts are enabled.
* Workflows use read-only tokens by default and pin third-party actions to
  commit SHAs.
* The workspace denies `unsafe` code (`unsafe_code = "deny"`).
