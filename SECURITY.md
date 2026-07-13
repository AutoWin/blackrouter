# Security Policy

## Supported Versions

Security fixes are applied to the latest release on the `main` branch.

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, report them privately using one of the following:

- GitHub's private vulnerability reporting (recommended): open a
  [security advisory](https://github.com/AutoWin/blackrouter/security/advisories/new)
  on the repository.
- Email the maintainers (see the repository's security tab for the current
  contact).

Please include:

- A description of the vulnerability and its impact.
- Steps to reproduce, or a proof of concept.
- Affected versions.

We will acknowledge your report as soon as possible and keep you informed of
progress toward a fix and disclosure.

## Security Notes for Operators

- BlackRouter is a gateway: it stores the provider credentials and gateway API
  keys you configure. Restrict access to the `/setup` UI and the control plane
  (`BLACKROUTER_CONTROL_API_ENABLED` / `BLACKROUTER_CONTROL_TOKEN`) in
  production.
- Run behind TLS (a reverse proxy such as Caddy/Nginx) so OAuth and credentials
  are never sent over plain HTTP.
- Set `BLACKROUTER_BASE_URL` to your public HTTPS URL so OAuth callbacks resolve
  correctly.
- Keep `data/` (the SQLite database) backed up and access-controlled.
