# Blood Package Registry Design

**Version**: 0.1.0
**Status**: Draft
**Related**: [Package Manifest](PACKAGE_MANIFEST.md)

## Overview

The Blood Package Registry is the central repository for distributing Blood packages. This document specifies the registry's architecture, API, security model, and operational considerations.

## Design Goals

1. **Security First**: All packages cryptographically signed, content-addressed
2. **High Availability**: Distributed architecture with mirrors and CDN
3. **Fast Resolution**: Efficient dependency resolution and download
4. **Transparency**: Full audit trail of package publications
5. **Reproducibility**: Content-addressed packages ensure identical builds

## Architecture

### High-Level Components

```
┌────────────────────────────────────────────────────────────────────────┐
│                           Blood Registry                                │
├────────────────────────────────────────────────────────────────────────┤
│                                                                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐                 │
│  │   API        │  │   Index      │  │   Storage    │                 │
│  │   Server     │  │   Server     │  │   (Blobs)    │                 │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘                 │
│         │                 │                 │                          │
│         └─────────────────┼─────────────────┘                          │
│                           │                                            │
│                    ┌──────▼──────┐                                     │
│                    │  PostgreSQL │                                     │
│                    │  Database   │                                     │
│                    └─────────────┘                                     │
│                                                                        │
├────────────────────────────────────────────────────────────────────────┤
│                         CDN / Mirrors                                   │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐                   │
│  │ Mirror  │  │ Mirror  │  │ Mirror  │  │ Mirror  │                   │
│  │ (US-E)  │  │ (US-W)  │  │ (EU)    │  │ (APAC)  │                   │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘                   │
└────────────────────────────────────────────────────────────────────────┘
```

### Component Responsibilities

#### API Server

- Authentication and authorization
- Package publication workflow
- User and team management
- API rate limiting
- Webhook notifications

#### Index Server

- Package metadata and search
- Dependency graph resolution
- Version compatibility checks
- Popular/trending packages
- Category and keyword indexing

#### Blob Storage

- Package tarballs (content-addressed)
- README and documentation
- Build artifacts (optional)
- Backup and replication

#### Database

- User accounts and tokens
- Package ownership and teams
- Publication history and audit log
- Download statistics

## Data Model

### Package

```sql
CREATE TABLE packages (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    downloads BIGINT NOT NULL DEFAULT 0,
    description TEXT,
    homepage VARCHAR(2048),
    repository VARCHAR(2048),
    documentation VARCHAR(2048),
    max_upload_size INTEGER DEFAULT 10485760,  -- 10 MB

    CONSTRAINT valid_name CHECK (name ~ '^[a-z][a-z0-9_-]*$')
);
```

### Version

```sql
CREATE TABLE versions (
    id SERIAL PRIMARY KEY,
    package_id INTEGER NOT NULL REFERENCES packages(id),
    version VARCHAR(255) NOT NULL,
    checksum VARCHAR(64) NOT NULL,  -- SHA-256
    content_hash VARCHAR(64) NOT NULL,  -- Blood content hash
    published_at TIMESTAMP NOT NULL DEFAULT NOW(),
    published_by INTEGER REFERENCES users(id),
    yanked BOOLEAN NOT NULL DEFAULT FALSE,
    yanked_reason TEXT,
    features JSONB NOT NULL DEFAULT '[]',
    edition VARCHAR(10),
    readme_path VARCHAR(255),
    license VARCHAR(255),

    UNIQUE(package_id, version),
    CONSTRAINT valid_version CHECK (version ~ '^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$')
);
```

### Dependency

```sql
CREATE TABLE dependencies (
    id SERIAL PRIMARY KEY,
    version_id INTEGER NOT NULL REFERENCES versions(id),
    dep_package_id INTEGER NOT NULL REFERENCES packages(id),
    req VARCHAR(255) NOT NULL,  -- Version requirement
    kind VARCHAR(20) NOT NULL,  -- normal, dev, build
    optional BOOLEAN NOT NULL DEFAULT FALSE,
    features JSONB NOT NULL DEFAULT '[]',
    target VARCHAR(255),  -- Target-specific dependency

    INDEX idx_deps_version (version_id),
    INDEX idx_deps_package (dep_package_id)
);
```

### User

```sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    username VARCHAR(255) NOT NULL UNIQUE,
    email VARCHAR(255) NOT NULL UNIQUE,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    password_hash VARCHAR(255) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    last_login TIMESTAMP,
    two_factor_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    totp_secret VARCHAR(255),

    CONSTRAINT valid_username CHECK (username ~ '^[a-z][a-z0-9_-]{2,}$')
);
```

### API Token

```sql
CREATE TABLE api_tokens (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    name VARCHAR(255) NOT NULL,
    token_hash VARCHAR(64) NOT NULL UNIQUE,  -- SHA-256 of token
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    last_used TIMESTAMP,
    expires_at TIMESTAMP,
    scopes JSONB NOT NULL DEFAULT '["publish"]',

    INDEX idx_tokens_user (user_id)
);
```

### Ownership

```sql
CREATE TABLE package_owners (
    package_id INTEGER NOT NULL REFERENCES packages(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    invited_by INTEGER REFERENCES users(id),

    PRIMARY KEY (package_id, user_id)
);
```

### Audit Log

```sql
CREATE TABLE audit_log (
    id SERIAL PRIMARY KEY,
    timestamp TIMESTAMP NOT NULL DEFAULT NOW(),
    action VARCHAR(50) NOT NULL,
    user_id INTEGER REFERENCES users(id),
    package_id INTEGER REFERENCES packages(id),
    version_id INTEGER REFERENCES versions(id),
    details JSONB,
    ip_address INET
);
```

## API Specification

### Authentication

All authenticated requests require an API token in the Authorization header:

```
Authorization: Bearer <token>
```

Tokens are generated with specific scopes:
- `publish`: Publish new versions
- `yank`: Yank existing versions
- `owner:read`: View owners
- `owner:write`: Add/remove owners

### Endpoints

#### Package Discovery

##### Search Packages

```
GET /api/v1/packages?q=<query>&page=<n>&per_page=<n>
```

Response:
```json
{
  "packages": [
    {
      "name": "json",
      "description": "JSON parser and serializer",
      "version": "1.2.3",
      "downloads": 15000,
      "updated_at": "2026-01-10T12:00:00Z"
    }
  ],
  "meta": {
    "total": 150,
    "page": 1,
    "per_page": 20
  }
}
```

##### Get Package Info

```
GET /api/v1/packages/<name>
```

Response:
```json
{
  "name": "json",
  "description": "JSON parser and serializer",
  "homepage": "https://github.com/blood-lang/json",
  "repository": "https://github.com/blood-lang/json",
  "documentation": "https://docs.blood-lang.org/json",
  "downloads": 15000,
  "versions": ["1.2.3", "1.2.2", "1.2.1", "1.2.0"],
  "owners": [
    {"username": "alice", "avatar": "https://..."}
  ],
  "categories": ["parsing", "serialization"],
  "keywords": ["json", "parser", "serde"]
}
```

##### Get Version Info

```
GET /api/v1/packages/<name>/<version>
```

Response:
```json
{
  "name": "json",
  "version": "1.2.3",
  "checksum": "sha256:abc123...",
  "content_hash": "blood:sha256:def456...",
  "published_at": "2026-01-10T12:00:00Z",
  "downloads": 5000,
  "yanked": false,
  "license": "MIT",
  "features": {
    "default": ["std"],
    "std": [],
    "derive": ["dep:json-derive"]
  },
  "dependencies": [
    {
      "name": "unicode",
      "req": "^1.0",
      "kind": "normal",
      "optional": false
    }
  ],
  "readme": "# JSON\n\nA fast JSON parser..."
}
```

#### Package Download

##### Download Package

```
GET /api/v1/packages/<name>/<version>/download
```

Response: Redirects to CDN URL for tarball download.

The tarball is named `<name>-<version>.tar.gz` and contains:
```
<name>-<version>/
├── Blood.toml
├── src/
│   └── lib.blood
├── README.md (optional)
└── .blood-checksum
```

#### Package Publication

##### Publish New Version

```
PUT /api/v1/packages/new
Authorization: Bearer <token>
Content-Type: application/octet-stream

<tarball data>
```

Response:
```json
{
  "package": {
    "name": "my-package",
    "version": "0.1.0"
  },
  "warnings": [
    "No `repository` specified in Blood.toml"
  ]
}
```

##### Yank Version

```
DELETE /api/v1/packages/<name>/<version>/yank
Authorization: Bearer <token>
```

Response:
```json
{
  "ok": true
}
```

##### Unyank Version

```
PUT /api/v1/packages/<name>/<version>/unyank
Authorization: Bearer <token>
```

Response:
```json
{
  "ok": true
}
```

#### Ownership Management

##### List Owners

```
GET /api/v1/packages/<name>/owners
Authorization: Bearer <token>
```

Response:
```json
{
  "owners": [
    {"username": "alice", "email": "alice@example.com"},
    {"username": "bob", "email": "bob@example.com"}
  ]
}
```

##### Add Owner

```
PUT /api/v1/packages/<name>/owners
Authorization: Bearer <token>
Content-Type: application/json

{"users": ["charlie"]}
```

##### Remove Owner

```
DELETE /api/v1/packages/<name>/owners
Authorization: Bearer <token>
Content-Type: application/json

{"users": ["bob"]}
```

#### User Management

##### Get Current User

```
GET /api/v1/me
Authorization: Bearer <token>
```

##### List User's Tokens

```
GET /api/v1/me/tokens
Authorization: Bearer <token>
```

##### Create New Token

```
POST /api/v1/me/tokens
Authorization: Bearer <token>
Content-Type: application/json

{
  "name": "CI Token",
  "scopes": ["publish"],
  "expires_at": "2027-01-01T00:00:00Z"
}
```

##### Revoke Token

```
DELETE /api/v1/me/tokens/<token_id>
Authorization: Bearer <token>
```

### Index API

For efficient dependency resolution, the registry provides a Git-based index.

#### Index Structure

```
index/
├── config.json
├── 1/              # Single-character package names
│   └── a
├── 2/              # Two-character names
│   └── ab
├── 3/              # Three-character names
│   └── a/
│       └── abc
└── ab/             # Four+ character names
    └── cd/
        └── abcdef
```

#### Package Index Entry

Each package file contains one JSON line per version:

```json
{"name":"json","vers":"1.2.3","deps":[{"name":"unicode","req":"^1.0","kind":"normal"}],"cksum":"abc123...","features":{"default":["std"]},"yanked":false}
```

#### Index Configuration

`config.json`:
```json
{
  "dl": "https://packages.blood-lang.org/api/v1/packages/{package}/{version}/download",
  "api": "https://packages.blood-lang.org"
}
```

## Security Model

### Package Verification

1. **Checksum Verification**: SHA-256 of tarball verified on download
2. **Content Hash**: Blood-specific content hash ensures source integrity
3. **Signature Verification**: Optional GPG/Sigstore signatures

```
┌─────────────────────────────────────────────────────────┐
│                    Package Verification                  │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Download         Verify           Verify        Extract│
│  Tarball    ──>   Checksum   ──>   Signature ──> Files │
│                   (SHA-256)        (optional)          │
│                                                         │
│                        │                                │
│                        v                                │
│              Verify Content Hash                        │
│              (blood:sha256:...)                         │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### Content Addressing

Blood packages use content-addressed hashes for reproducibility:

```
content_hash = blood:sha256:hash(
    canonical_blood_toml ||
    sorted_source_files.map(|f| hash(f.path || f.content))
)
```

This ensures identical source code produces identical hashes regardless of:
- Publication time
- File system metadata
- Compression method

### Token Security

1. **Hashed Storage**: Tokens stored as SHA-256 hashes
2. **Scoped Access**: Tokens limited to specific operations
3. **Expiration**: Optional expiration dates
4. **Rate Limiting**: Per-token rate limits prevent abuse

### Publication Security

1. **Two-Factor Authentication**: Required for publishing
2. **Ownership Verification**: Only owners can publish
3. **Audit Logging**: All actions logged with IP address
4. **Yank Protection**: Yanking requires confirmation

### Vulnerability Handling

Integration with security advisory system (see ECO-010):

1. **Advisory Database**: Known vulnerabilities tracked
2. **Automated Alerts**: Users notified of vulnerable dependencies
3. **Yank Recommendations**: Severely vulnerable versions yanked

## Operational Considerations

### Rate Limiting

| Endpoint | Anonymous | Authenticated | With API Token |
|----------|-----------|---------------|----------------|
| Search | 30/min | 100/min | 100/min |
| Package Info | 60/min | 200/min | 500/min |
| Download | 100/min | 500/min | 1000/min |
| Publish | N/A | 10/hour | 10/hour |

### Caching Strategy

```
┌─────────────────────────────────────────────────────────────────┐
│                      Caching Architecture                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Client        CDN Edge        Origin          Database         │
│    │               │              │                │            │
│    │  Request      │              │                │            │
│    │──────────────>│              │                │            │
│    │               │              │                │            │
│    │   Cache Hit   │              │                │            │
│    │<──────────────│              │                │            │
│    │               │              │                │            │
│    │  Cache Miss   │   Request    │                │            │
│    │               │─────────────>│                │            │
│    │               │              │   Query        │            │
│    │               │              │───────────────>│            │
│    │               │              │<───────────────│            │
│    │               │<─────────────│                │            │
│    │<──────────────│              │                │            │
│    │               │              │                │            │
└─────────────────────────────────────────────────────────────────┘
```

Cache TTLs:
- Package tarballs: Immutable (infinite cache)
- Package metadata: 5 minutes
- Search results: 1 minute
- Index: 1 minute (Git fetch)

### Mirroring

Official mirrors can be run by the community:

```bash
# Mirror configuration
blood registry mirror \
  --upstream https://packages.blood-lang.org \
  --listen 0.0.0.0:8080 \
  --storage /var/blood-mirror
```

Mirror requirements:
- Full index replication
- Tarball caching (on-demand or full)
- Regular synchronization (every minute)

### Monitoring

Key metrics:
- Request latency (p50, p95, p99)
- Download bandwidth
- Publication success rate
- Error rates by endpoint
- Database connection pool usage
- Storage utilization

### Disaster Recovery

1. **Database Backups**: Hourly snapshots, 30-day retention
2. **Blob Storage**: Cross-region replication
3. **Index**: Git repository with multiple remotes
4. **Recovery Time Objective**: < 1 hour
5. **Recovery Point Objective**: < 1 hour

## CLI Integration

### Login

```bash
blood login
# Opens browser for OAuth flow
# Stores token in ~/.blood/credentials.toml
```

### Publish

```bash
blood publish
# 1. Builds package
# 2. Runs pre-publish checks
# 3. Uploads to registry
# 4. Verifies publication
```

### Ownership

```bash
blood owner list my-package
blood owner add my-package alice
blood owner remove my-package bob
```

### Token Management

```bash
blood token new --name "CI" --scopes publish --expires 365d
blood token list
blood token revoke <token-id>
```

## Alternative Registries

Blood supports alternative registries for private packages:

```toml
# ~/.blood/config.toml
[registries]
my-company = { index = "https://packages.company.com/index" }

# Blood.toml
[dependencies]
internal-lib = { version = "1.0", registry = "my-company" }
```

## Migration from Other Ecosystems

### From Cargo

```bash
blood import-cargo ./Cargo.toml
# Generates Blood.toml from Cargo manifest
```

### From npm

```bash
blood import-npm ./package.json
# Generates Blood.toml from npm manifest
```

## Future Enhancements

1. **Verified Publishers**: Badge for verified organizations
2. **Build Provenance**: SLSA build attestations
3. **Dependency Graph API**: Query transitive dependencies
4. **Automated Security Scanning**: Scan on publish
5. **Documentation Hosting**: Auto-generated API docs
6. **Binary Artifacts**: Pre-compiled platform binaries

## Version History

| Version | Changes |
|---------|---------|
| 0.1.0 | Initial specification |

---

*This specification is subject to change as Blood's package ecosystem matures.*
