# Blood Security Advisory System

**Version**: 1.0
**Status**: Draft
**Related**: [Package Registry](../tooling/PACKAGE_REGISTRY.md), [Security Model](../guides/SECURITY_MODEL.md)

## Overview

The Blood Security Advisory System provides a centralized database and tooling for tracking, reporting, and remediating security vulnerabilities in Blood packages. This system enables developers to identify vulnerable dependencies and take appropriate action.

## Goals

1. **Comprehensive Coverage**: Track all known vulnerabilities in the ecosystem
2. **Timely Notifications**: Alert maintainers and users quickly
3. **Actionable Guidance**: Provide clear remediation steps
4. **Transparency**: Public database with full disclosure
5. **Integration**: Seamless integration with Blood tooling

## Architecture

### System Components

```
┌───────────────────────────────────────────────────────────────────┐
│                    Security Advisory System                        │
├───────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐            │
│  │  Advisory    │  │  Scanner     │  │  Notifier    │            │
│  │  Database    │  │  Service     │  │  Service     │            │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘            │
│         │                 │                 │                     │
│         └─────────────────┼─────────────────┘                     │
│                           │                                       │
│                    ┌──────▼──────┐                                │
│                    │   API       │                                │
│                    │   Gateway   │                                │
│                    └─────────────┘                                │
│                           │                                       │
├───────────────────────────┼───────────────────────────────────────┤
│                           │                                       │
│  ┌──────────────┐  ┌──────▼──────┐  ┌──────────────┐             │
│  │  blood CLI   │  │  Registry   │  │  GitHub      │             │
│  │  audit       │  │  Integration│  │  Action      │             │
│  └──────────────┘  └─────────────┘  └──────────────┘             │
│                                                                   │
└───────────────────────────────────────────────────────────────────┘
```

### Advisory Database

A Git repository containing all security advisories in a structured format.

#### Repository Structure

```
blood-advisory-db/
├── advisories/
│   ├── json/
│   │   ├── BLOOD-2026-0001.toml
│   │   └── BLOOD-2026-0002.toml
│   ├── http/
│   │   └── BLOOD-2026-0003.toml
│   └── crypto/
│       └── BLOOD-2026-0004.toml
├── collections/
│   └── rust-advisories.toml     # Imported from other ecosystems
├── withdrawn/
│   └── BLOOD-2026-0005.toml     # False positives
└── README.md
```

## Advisory Format

### Advisory File Structure

```toml
# advisories/json/BLOOD-2026-0001.toml

[advisory]
id = "BLOOD-2026-0001"
package = "json"
date = "2026-01-10"
title = "Buffer overflow in JSON parser"
description = """
A buffer overflow vulnerability exists in the json package's
parser when handling deeply nested arrays. An attacker can craft
malicious JSON input to cause memory corruption.
"""
url = "https://blood-lang.org/advisories/BLOOD-2026-0001"
categories = ["memory-corruption"]
keywords = ["buffer-overflow", "parser"]

[versions]
patched = [">=1.2.4"]
unaffected = ["<1.0.0"]
# Affected: 1.0.0 <= version < 1.2.4

[severity]
level = "high"
cvss_v3 = "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H"

[references]
cve = "CVE-2026-12345"
urls = [
    "https://github.com/blood-lang/json/issues/42",
    "https://github.com/blood-lang/json/commit/abc123",
]

[affected]
# Functions/modules specifically affected
functions = ["json::parse", "json::Parser::parse_array"]
```

### Severity Levels

| Level | CVSS Score | Description |
|-------|------------|-------------|
| Critical | 9.0-10.0 | Immediate exploitation risk, severe impact |
| High | 7.0-8.9 | Significant security impact |
| Medium | 4.0-6.9 | Moderate security impact |
| Low | 0.1-3.9 | Minor security impact |
| None | 0.0 | Informational only |

### Categories

Standardized vulnerability categories:

| Category | Description |
|----------|-------------|
| `memory-corruption` | Buffer overflows, use-after-free, etc. |
| `denial-of-service` | Resource exhaustion, crashes |
| `code-execution` | Remote or local code execution |
| `information-disclosure` | Data leaks, timing attacks |
| `privilege-escalation` | Unauthorized access elevation |
| `injection` | Command, SQL, or other injection |
| `cryptography` | Weak algorithms, implementation flaws |
| `authentication` | Auth bypass, session issues |
| `file-access` | Path traversal, unauthorized file access |

## API Specification

### Query Advisories

```
GET /api/v1/advisories?package=<name>&version=<version>
```

Response:
```json
{
  "advisories": [
    {
      "id": "BLOOD-2026-0001",
      "package": "json",
      "title": "Buffer overflow in JSON parser",
      "severity": "high",
      "patched_versions": [">=1.2.4"],
      "url": "https://blood-lang.org/advisories/BLOOD-2026-0001"
    }
  ]
}
```

### Get Advisory Details

```
GET /api/v1/advisories/BLOOD-2026-0001
```

Response:
```json
{
  "id": "BLOOD-2026-0001",
  "package": "json",
  "date": "2026-01-10",
  "title": "Buffer overflow in JSON parser",
  "description": "A buffer overflow vulnerability...",
  "severity": {
    "level": "high",
    "cvss_v3": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H"
  },
  "versions": {
    "patched": [">=1.2.4"],
    "unaffected": ["<1.0.0"]
  },
  "references": {
    "cve": "CVE-2026-12345",
    "urls": ["https://github.com/blood-lang/json/issues/42"]
  },
  "affected_functions": ["json::parse"]
}
```

### Bulk Audit

```
POST /api/v1/audit
Content-Type: application/json

{
  "packages": [
    {"name": "json", "version": "1.2.0"},
    {"name": "http", "version": "2.0.0"}
  ]
}
```

Response:
```json
{
  "vulnerabilities": [
    {
      "package": "json",
      "version": "1.2.0",
      "advisories": ["BLOOD-2026-0001"]
    }
  ],
  "warnings": [],
  "info": {
    "advisories_checked": 150,
    "database_updated": "2026-01-13T12:00:00Z"
  }
}
```

### Report New Vulnerability

```
POST /api/v1/report
Authorization: Bearer <token>
Content-Type: application/json

{
  "package": "vulnerable-pkg",
  "title": "Security issue in X",
  "description": "Detailed description...",
  "affected_versions": ">=1.0.0, <2.0.0",
  "severity_estimate": "high",
  "contact_email": "reporter@example.com"
}
```

Response:
```json
{
  "report_id": "RPT-2026-0042",
  "status": "received",
  "message": "Thank you for your report. We will review it within 48 hours."
}
```

## CLI Integration

### blood audit

The primary command for checking vulnerabilities:

```bash
# Audit current project
$ blood audit

    Scanning Blood.lock for vulnerabilities...

    Vulnerability found!

    ╭─ BLOOD-2026-0001 ──────────────────────────────────────────────╮
    │ Buffer overflow in JSON parser                                 │
    │                                                                │
    │ Severity: HIGH                                                 │
    │ Package:  json 1.2.0                                          │
    │ Patched:  >=1.2.4                                             │
    │                                                                │
    │ A buffer overflow vulnerability exists in the json package's   │
    │ parser when handling deeply nested arrays.                     │
    │                                                                │
    │ More info: https://blood-lang.org/advisories/BLOOD-2026-0001  │
    ╰───────────────────────────────────────────────────────────────╯

    Found 1 vulnerability (1 high, 0 medium, 0 low)

    Run `blood update json` to update to a patched version.
```

### Command Options

```bash
# Audit with specific database
blood audit --database /path/to/advisory-db

# Audit and fail CI on vulnerabilities
blood audit --deny critical --deny high

# Show all advisories, including informational
blood audit --verbose

# Output in JSON format
blood audit --format json

# Ignore specific advisories
blood audit --ignore BLOOD-2026-0001

# Audit specific packages only
blood audit --package json --package http

# Update advisory database
blood audit --update-db
```

### Configuration

In `Blood.toml`:

```toml
[audit]
# Fail build on these severity levels
deny = ["critical", "high"]

# Warn but don't fail on these
warn = ["medium", "low"]

# Ignore specific advisories (with justification)
ignore = [
    { id = "BLOOD-2026-0001", reason = "Not exploitable in our use case" }
]

# Alternative advisory database
database = "https://company.com/internal-advisories"
```

## Notification System

### Email Notifications

Package owners receive email notifications:

```
Subject: [Blood Security] Vulnerability in your package: json

A security vulnerability has been reported in your package.

Package: json
Advisory: BLOOD-2026-0001
Severity: HIGH
Title: Buffer overflow in JSON parser

Action Required:
1. Review the advisory at https://blood-lang.org/advisories/BLOOD-2026-0001
2. Develop and test a fix
3. Publish a patched version
4. The advisory will be updated once the fix is available

If you believe this is a false positive, please reply to this email
with details.

---
Blood Security Team
security@blood-lang.org
```

### Webhook Notifications

Projects can configure webhooks:

```toml
# Blood.toml
[webhooks.security]
url = "https://hooks.slack.com/services/..."
events = ["advisory.new", "advisory.updated"]
```

Webhook payload:

```json
{
  "event": "advisory.new",
  "advisory": {
    "id": "BLOOD-2026-0001",
    "package": "json",
    "severity": "high",
    "title": "Buffer overflow in JSON parser"
  },
  "affected_projects": ["my-app", "my-lib"],
  "timestamp": "2026-01-10T12:00:00Z"
}
```

### GitHub Integration

GitHub Action for continuous monitoring:

```yaml
# .github/workflows/security.yml
name: Security Audit

on:
  push:
    branches: [main]
  schedule:
    - cron: '0 0 * * *'  # Daily

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - uses: blood-lang/audit-action@v1
        with:
          deny: critical,high
          create-issues: true
```

## Vulnerability Lifecycle

### 1. Discovery

```
Reporter                    Security Team
   │                             │
   │  Report vulnerability       │
   │────────────────────────────>│
   │                             │
   │  Acknowledge receipt        │
   │<────────────────────────────│
   │                             │
```

### 2. Triage

```
Security Team               Package Maintainer
   │                             │
   │  Validate report            │
   │                             │
   │  Notify maintainer          │
   │────────────────────────────>│
   │                             │
   │  Confirm/provide details    │
   │<────────────────────────────│
   │                             │
```

### 3. Fix Development

```
Package Maintainer          Security Team
   │                             │
   │  Develop patch              │
   │                             │
   │  Request review             │
   │────────────────────────────>│
   │                             │
   │  Approve patch              │
   │<────────────────────────────│
   │                             │
```

### 4. Coordinated Disclosure

```
                                Timeline
Day 0:   Report received
Day 1:   Triage complete, maintainer notified
Day 7:   Patch developed
Day 14:  Advisory prepared (embargoed)
Day 21:  Patch published, advisory released
Day 28:  CVE assigned
Day 90:  Full public disclosure
```

### 5. Post-Disclosure

- Monitor for exploitation attempts
- Track patch adoption
- Update advisory if needed
- Archive after sufficient time

## Advisory Database Management

### Contributing Advisories

```bash
# Fork and clone the database
git clone https://github.com/blood-lang/advisory-db
cd advisory-db

# Create new advisory
mkdir -p advisories/vulnerable-pkg
cat > advisories/vulnerable-pkg/BLOOD-2026-XXXX.toml << EOF
[advisory]
id = "BLOOD-2026-XXXX"
...
EOF

# Validate advisory
blood advisory validate advisories/vulnerable-pkg/BLOOD-2026-XXXX.toml

# Submit pull request
git checkout -b advisory/BLOOD-2026-XXXX
git add .
git commit -m "Add advisory BLOOD-2026-XXXX"
git push origin advisory/BLOOD-2026-XXXX
```

### Validation Rules

Advisory files must pass validation:

1. **Required Fields**: id, package, date, title, description
2. **Version Syntax**: patched/unaffected must be valid semver ranges
3. **Severity**: If CVSS provided, must be valid CVSS string
4. **Package Exists**: Package must exist in registry
5. **ID Format**: Must match `BLOOD-YYYY-NNNN`

### Withdrawing Advisories

For false positives:

```toml
# withdrawn/BLOOD-2026-0005.toml

[advisory]
id = "BLOOD-2026-0005"
package = "false-positive-pkg"
withdrawn = "2026-01-15"
reason = "Investigation determined no security impact"
original_reporter = "reporter@example.com"
```

## Integration with Registry

### Yank Recommendations

Severely vulnerable versions may be recommended for yanking:

```toml
[yank_recommendation]
advisory = "BLOOD-2026-0001"
versions = ["1.0.0", "1.1.0", "1.2.0"]
reason = "Critical vulnerability with active exploitation"
```

### Download Warnings

When downloading vulnerable versions:

```
warning: json@1.2.0 has known vulnerabilities

  BLOOD-2026-0001: Buffer overflow in JSON parser (high)
                   Patched in: >=1.2.4

  Consider updating to json@1.2.4 or later.
```

### Publication Checks

New versions are scanned for known vulnerable dependencies:

```
warning: your package depends on vulnerable packages

  json@1.2.0: BLOOD-2026-0001 (high)

  Consider updating dependencies before publishing.
  Use --allow-vulnerable to publish anyway.
```

## Metrics and Reporting

### Dashboard Metrics

- Total advisories by severity
- Time to patch (median, p95)
- Most vulnerable packages
- Advisory trend over time
- Patch adoption rate

### Public Reports

Quarterly security reports including:

- New advisories published
- Vulnerabilities by category
- Ecosystem health metrics
- Notable incidents

## Best Practices

### For Package Maintainers

1. **Enable Security Alerts**: Subscribe to notifications for your packages
2. **Respond Quickly**: Acknowledge reports within 48 hours
3. **Coordinate Disclosure**: Work with security team on timing
4. **Document Clearly**: Provide migration guides for breaking changes
5. **Backport Patches**: Consider patching older supported versions

### For Application Developers

1. **Regular Audits**: Run `blood audit` in CI
2. **Update Promptly**: Apply security updates quickly
3. **Monitor Advisories**: Subscribe to announcements
4. **Pin Dependencies**: Use lock files for reproducibility
5. **Review Transitive Deps**: Check entire dependency tree

### For Security Researchers

1. **Report Responsibly**: Follow coordinated disclosure
2. **Provide Details**: Include reproduction steps
3. **Suggest Fixes**: Help with remediation if possible
4. **Allow Time**: Give maintainers time to fix before disclosure
5. **Credit Attribution**: Let us know how to credit you

## API Rate Limits

| Endpoint | Rate Limit |
|----------|------------|
| GET /advisories | 100/min |
| POST /audit | 30/min |
| POST /report | 10/hour |

## Version History

| Version | Changes |
|---------|---------|
| 1.0 | Initial specification |

---

*This system is designed to protect the Blood ecosystem. Report vulnerabilities to security@blood-lang.org.*
