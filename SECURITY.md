# Security Policy

## Supported versions

GraphNight is **alpha**. There is no supported production release yet. Security fixes are applied on a best-effort basis on `main`.

## Alpha risk summary

Until the LAUNCH.md production (B1) checklist is complete:

- GraphQL is unauthenticated by default
- Datasource connection strings can be supplied via API mutations
- Row-level security / session policies are **not** enforced on the live query path
- Do not expose a GraphNight server to the public internet or attach production credentials

## Reporting a vulnerability

Please **do not** open a public GitHub issue for sensitive reports.

Email: **nirunitk@gmail.com** with:

- Description of the issue
- Steps to reproduce / proof of concept (non-destructive)
- Affected commit SHA or release tag if known

We will acknowledge reports as soon as practical and coordinate disclosure after a fix is available on `main`.
