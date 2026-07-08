# Security Policy

## Supported Versions

Security fixes target the latest released `vifu` version.

## Reporting a Vulnerability

Please report suspected vulnerabilities privately through GitHub Security
Advisories for `vifu-labs/vifu`.

Do not open public issues that include:

- local access tokens
- OpenClaw Gateway tokens or passwords
- logs containing sensitive data

## Security Principles

- The local connector must not upload OpenClaw credentials.
- The local connector must not expose a public listener by default.
- New network surfaces require tests and documentation before release.
