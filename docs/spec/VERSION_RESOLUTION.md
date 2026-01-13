# Blood Version Resolution Algorithm

**Version**: 1.0
**Status**: Draft
**Related**: [Package Manifest](PACKAGE_MANIFEST.md), [Package Registry](PACKAGE_REGISTRY.md)

## Overview

Version resolution is the process of selecting compatible package versions given a set of dependency requirements. Blood uses a SAT-solver-based approach similar to Pub (Dart) and Cargo, with extensions for content-addressed packages.

## Goals

1. **Reproducibility**: Same inputs produce identical resolution
2. **Completeness**: Find a solution if one exists
3. **Preference**: Prefer newer compatible versions
4. **Performance**: Resolve quickly even with many dependencies
5. **Determinism**: Always produce the same result for same inputs

## Semantic Versioning

Blood follows Semantic Versioning 2.0.0:

```
MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]
```

- **MAJOR**: Incompatible API changes
- **MINOR**: Backwards-compatible new features
- **PATCH**: Backwards-compatible bug fixes
- **PRERELEASE**: Pre-release version (e.g., `-alpha.1`)
- **BUILD**: Build metadata (ignored in resolution)

### Version Precedence

Versions are compared by precedence:

1. Major > Minor > Patch (numerically)
2. Release > Pre-release
3. Pre-release identifiers compared lexicographically

Examples (lowest to highest):
```
1.0.0-alpha < 1.0.0-alpha.1 < 1.0.0-beta < 1.0.0 < 1.0.1 < 1.1.0 < 2.0.0
```

## Version Requirements

### Syntax

| Syntax | Name | Meaning |
|--------|------|---------|
| `1.2.3` | Exact | Exactly version 1.2.3 |
| `^1.2.3` | Caret | >=1.2.3, <2.0.0 |
| `~1.2.3` | Tilde | >=1.2.3, <1.3.0 |
| `1.2.*` | Wildcard | >=1.2.0, <1.3.0 |
| `>=1.2.3` | Greater-equal | >=1.2.3 |
| `>1.2.3` | Greater | >1.2.3 |
| `<=1.2.3` | Less-equal | <=1.2.3 |
| `<1.2.3` | Less | <1.2.3 |
| `>=1.0, <2.0` | Range | Combined constraints |
| `*` | Any | Any version |

### Caret Requirements (Default)

The caret (`^`) is the default and most common requirement:

```
^1.2.3  →  >=1.2.3, <2.0.0
^0.2.3  →  >=0.2.3, <0.3.0   (0.x is special)
^0.0.3  →  >=0.0.3, <0.0.4   (0.0.x is extra special)
```

Rationale: Allows compatible updates while preventing breaking changes.

### Tilde Requirements

The tilde (`~`) is more restrictive:

```
~1.2.3  →  >=1.2.3, <1.3.0
~1.2    →  >=1.2.0, <1.3.0
~1      →  >=1.0.0, <2.0.0
```

Rationale: Only patch-level updates, useful when minor versions may break.

## Resolution Algorithm

### Problem Definition

Given:
- Root package with direct dependencies
- Dependency graph from registry
- Set of constraints

Find:
- A version for each required package
- All constraints satisfied
- No conflicts

### Algorithm: PubGrub

Blood uses the PubGrub algorithm (from Dart's Pub package manager), which provides:

- **Completeness**: Finds solution if one exists
- **Conflict-Driven**: Learns from conflicts to prune search space
- **Optimal Conflict Messages**: Explains why resolution failed

#### Data Structures

```blood
// A term is a positive or negative requirement
struct Term {
    package: String,
    constraint: VersionConstraint,
    positive: bool,  // true = must satisfy, false = must not satisfy
}

// An incompatibility explains why certain versions cannot be used together
struct Incompatibility {
    terms: Vec<Term>,
    cause: IncompatibilityCause,
}

enum IncompatibilityCause {
    Root,                           // Root package requirement
    Dependency(String, Version),    // Package at version has dependency
    NoVersions,                     // No versions satisfy constraint
    Conflict(Box<Incompatibility>, Box<Incompatibility>), // Derived
}

// Partial solution during resolution
struct PartialSolution {
    assignments: Vec<Assignment>,
    decisions: Vec<(String, Version)>,
}

struct Assignment {
    package: String,
    constraint: VersionConstraint,
    decision_level: usize,
    cause: Option<Incompatibility>,
}
```

#### Core Algorithm

```blood
fn resolve(root: Package, registry: Registry) -> Result<Solution, ResolutionError> {
    let mut solution = PartialSolution::new();
    let mut incompatibilities = Vec::new();

    // Add root dependencies as initial incompatibilities
    for dep in root.dependencies.iter() {
        incompatibilities.push(Incompatibility {
            terms: vec![
                Term::negative("root", any_version()),
                Term::positive(&dep.name, dep.constraint.complement()),
            ],
            cause: IncompatibilityCause::Root,
        });
    }

    loop {
        // Unit propagation: derive assignments from incompatibilities
        match propagate(&mut solution, &incompatibilities) {
            PropagateResult::Ok => {}
            PropagateResult::Conflict(incomp) => {
                // Analyze conflict and learn new incompatibility
                let (new_incomp, backtrack_level) = analyze_conflict(&solution, &incomp);

                if backtrack_level == 0 {
                    // No solution exists
                    return Err(build_error(&new_incomp));
                }

                incompatibilities.push(new_incomp);
                solution.backtrack_to(backtrack_level);
            }
        }

        // Select next package to decide
        match select_next_package(&solution) {
            Some(package) => {
                // Choose best version satisfying constraints
                let version = choose_version(&package, &solution, &registry);
                solution.decide(&package, version);

                // Add incompatibilities from chosen version's dependencies
                let deps = registry.dependencies(&package, version);
                for dep in deps.iter() {
                    incompatibilities.push(dependency_incompatibility(
                        &package, version, &dep
                    ));
                }
            }
            None => {
                // All packages decided
                return Ok(solution.to_solution());
            }
        }
    }
}
```

#### Unit Propagation

```blood
fn propagate(
    solution: &mut PartialSolution,
    incompatibilities: &Vec<Incompatibility>,
) -> PropagateResult {
    let mut changed = true;

    while changed {
        changed = false;

        for incomp in incompatibilities.iter() {
            match analyze_incompatibility(incomp, solution) {
                // All terms satisfied - conflict!
                IncompResult::Conflict => {
                    return PropagateResult::Conflict(incomp.clone());
                }
                // One term unsatisfied, others satisfied - must derive
                IncompResult::Unit(term) => {
                    solution.derive(term.package, term.constraint.negate(), incomp);
                    changed = true;
                }
                // Multiple unsatisfied - cannot derive yet
                IncompResult::Inconclusive => {}
            }
        }
    }

    PropagateResult::Ok
}
```

#### Version Selection

When selecting a version, prefer:

1. Already-selected version (if compatible)
2. Highest compatible non-prerelease version
3. Highest compatible prerelease version

```blood
fn choose_version(
    package: &str,
    solution: &PartialSolution,
    registry: &Registry,
) -> Version {
    let constraint = solution.constraint_for(package);
    let versions = registry.versions(package);

    // Filter to compatible versions
    let compatible: Vec<Version> = versions.iter()
        .filter(|v| constraint.allows(v))
        .cloned()
        .collect();

    // Prefer non-prerelease
    let stable: Vec<Version> = compatible.iter()
        .filter(|v| !v.is_prerelease())
        .cloned()
        .collect();

    if !stable.is_empty() {
        return stable.iter().max().unwrap().clone();
    }

    // Fall back to prerelease
    compatible.iter().max().unwrap().clone()
}
```

#### Conflict Analysis

When a conflict is found, analyze it to learn a new incompatibility:

```blood
fn analyze_conflict(
    solution: &PartialSolution,
    conflict: &Incompatibility,
) -> (Incompatibility, usize) {
    let mut current = conflict.clone();
    let mut backtrack_level = 0;

    while !is_root_cause(&current, solution) {
        // Find term that was most recently decided
        let (term, assn) = most_recent_satisfier(&current, solution);

        // If assignment was derived, resolve with its cause
        if let Some(cause) = &assn.cause {
            current = resolve_incompatibilities(&current, cause, &term);
        } else {
            // Decision - need to backtrack
            let prev_level = previous_decision_level(&current, solution);
            backtrack_level = prev_level;
            break;
        }
    }

    (current, backtrack_level)
}
```

## Lock File Generation

After successful resolution, generate `Blood.lock`:

```blood
fn generate_lockfile(solution: &Solution) -> Lockfile {
    let mut packages = Vec::new();

    // Sort packages for deterministic output
    let mut sorted: Vec<_> = solution.packages.iter().collect();
    sorted.sort_by_key(|(name, _)| *name);

    for (name, version) in sorted.iter() {
        let info = registry.info(name, version);
        packages.push(LockPackage {
            name: name.clone(),
            version: version.clone(),
            source: info.source,
            checksum: info.checksum,
            dependencies: solution.dependencies_of(name),
        });
    }

    Lockfile { packages }
}
```

### Lock File Format

```toml
# Blood.lock
# This file is auto-generated. Do not edit.

[[package]]
name = "json"
version = "1.2.3"
source = "registry+https://packages.blood-lang.org"
checksum = "sha256:abc123..."
dependencies = [
    "unicode 1.0.0",
]

[[package]]
name = "unicode"
version = "1.0.0"
source = "registry+https://packages.blood-lang.org"
checksum = "sha256:def456..."
```

## Content-Addressed Resolution

Blood extends standard resolution with content-addressed packages:

### Content Hash Verification

When a dependency specifies a content hash:

```toml
[dependencies]
verified = { hash = "blood:sha256:abc123...", version = "1.0.0" }
```

Resolution verifies:
1. Version 1.0.0 exists
2. Version's content hash matches specified hash
3. Fails if hash doesn't match (potential supply chain attack)

### Resolution with Hashes

```blood
fn resolve_with_hash(
    package: &str,
    version: &Version,
    expected_hash: &str,
    registry: &Registry,
) -> Result<(), HashMismatch> {
    let actual_hash = registry.content_hash(package, version);

    if actual_hash != expected_hash {
        return Err(HashMismatch {
            package: package.to_string(),
            version: version.clone(),
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(())
}
```

## Workspace Resolution

For workspaces with multiple packages:

1. Collect all dependencies from all members
2. Resolve once for entire workspace
3. Share versions across all members

```blood
fn resolve_workspace(workspace: &Workspace, registry: &Registry) -> Result<Solution, Error> {
    // Merge all dependencies
    let mut all_deps = Vec::new();
    for member in workspace.members.iter() {
        all_deps.extend(member.dependencies.clone());
    }

    // Deduplicate, taking strictest constraint
    let merged = merge_dependencies(&all_deps);

    // Resolve unified dependency set
    resolve_dependencies(&merged, registry)
}
```

## Error Messages

Resolution failures produce detailed error messages:

### Version Conflict

```
error: failed to resolve dependencies

  Because my-app depends on json ^1.0 and http depends on json ^2.0,
  json ^1.0 and json ^2.0 are incompatible.

  And because my-app depends on http ^1.0, version solving failed.

  Possible solutions:
  1. Upgrade json requirement to ^2.0
  2. Downgrade http to a version compatible with json ^1.0
```

### No Matching Version

```
error: failed to resolve dependencies

  Because no version of json matches >=3.0.0 and my-app depends on
  json >=3.0.0, version solving failed.

  json versions available: 1.0.0, 1.1.0, 2.0.0, 2.1.0

  Did you mean json ^2.0?
```

### Yanked Version

```
warning: selected version json@1.0.0 has been yanked

  Reason: Security vulnerability CVE-2026-1234

  Consider upgrading to json@1.0.1 or later.
```

## Performance Optimizations

### Caching

1. **Version Cache**: Cache available versions per package
2. **Dependency Cache**: Cache dependencies per version
3. **Incompatibility Cache**: Persist learned incompatibilities

### Parallel Fetching

Fetch package metadata in parallel during resolution:

```blood
async fn prefetch_versions(
    packages: &[String],
    registry: &Registry,
) -> HashMap<String, Vec<Version>> {
    let futures: Vec<_> = packages.iter()
        .map(|p| async { (p.clone(), registry.versions(p).await) })
        .collect();

    join_all(futures).await.into_iter().collect()
}
```

### Index Optimization

Use registry index for fast lookups:

```blood
fn load_index_entry(package: &str, index: &Index) -> Vec<IndexVersion> {
    let path = index_path(package);
    let content = index.read(&path)?;

    content.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
```

## Edge Cases

### Circular Dependencies

Blood prohibits circular dependencies:

```
error: circular dependency detected

  my-app -> lib-a -> lib-b -> my-app

  Circular dependencies are not allowed.
```

### Optional Dependencies

Optional dependencies only included when feature is enabled:

```blood
fn collect_dependencies(
    package: &Package,
    features: &HashSet<String>,
) -> Vec<Dependency> {
    let mut deps = Vec::new();

    for dep in package.dependencies.iter() {
        if dep.optional {
            // Check if feature enables this dependency
            let feature_name = format!("dep:{}", dep.name);
            if !features.contains(&feature_name) {
                continue;
            }
        }
        deps.push(dep.clone());
    }

    deps
}
```

### Target-Specific Dependencies

Only include dependencies matching current target:

```blood
fn filter_by_target(deps: &[Dependency], target: &Target) -> Vec<Dependency> {
    deps.iter()
        .filter(|d| match &d.target {
            Some(t) => target.matches(t),
            None => true,
        })
        .cloned()
        .collect()
}
```

## CLI Commands

### Update Dependencies

```bash
# Update all dependencies to latest compatible
blood update

# Update specific package
blood update json

# Update to latest, ignoring semver
blood update --breaking json
```

### Check Resolution

```bash
# Verify lock file is up-to-date
blood check

# Show dependency tree
blood tree

# Show why a package is included
blood tree --invert json
```

### Audit

```bash
# Check for security advisories
blood audit

# Check for outdated dependencies
blood outdated
```

## Testing Resolution

The resolver includes comprehensive tests:

```blood
#[test]
fn test_simple_resolution() {
    let registry = mock_registry! {
        "a" => ["1.0.0", "1.1.0", "2.0.0"],
        "b" => ["1.0.0" => { "a" => "^1.0" }],
    };

    let root = package! {
        dependencies: { "b" => "^1.0" }
    };

    let solution = resolve(&root, &registry).unwrap();

    assert_eq!(solution.get("a"), Some(&version("1.1.0")));
    assert_eq!(solution.get("b"), Some(&version("1.0.0")));
}

#[test]
fn test_conflict_detection() {
    let registry = mock_registry! {
        "a" => ["1.0.0", "2.0.0"],
        "b" => ["1.0.0" => { "a" => "^1.0" }],
        "c" => ["1.0.0" => { "a" => "^2.0" }],
    };

    let root = package! {
        dependencies: {
            "b" => "^1.0",
            "c" => "^1.0",
        }
    };

    let err = resolve(&root, &registry).unwrap_err();
    assert!(err.message.contains("incompatible"));
}
```

## Version History

| Version | Changes |
|---------|---------|
| 1.0 | Initial specification |

---

*This specification defines the core resolution algorithm. Implementation details may vary.*
