# Blood Project Governance

This document describes the governance model for the Blood programming language project.

## Overview

Blood is currently a single-maintainer open source project. This governance document establishes the foundation for growing into a sustainable community-driven project.

## Roles

### Project Lead

The Project Lead has final authority over:
- Language design decisions
- Major architectural changes
- Release planning
- Community standards

**Current Project Lead**: [To be designated]

### Maintainers

Maintainers are trusted contributors who can:
- Merge pull requests
- Triage issues
- Participate in design discussions
- Mentor new contributors

**Becoming a Maintainer:**
1. Demonstrate sustained, quality contributions
2. Show good judgment in code reviews
3. Understand the project's design philosophy
4. Be nominated by existing maintainer
5. Approved by Project Lead

### Contributors

Anyone who contributes to the project:
- Code contributions
- Documentation
- Bug reports
- Feature suggestions
- Community support

All contributors must follow the [Contributor Covenant Code of Conduct](https://www.contributor-covenant.org/).

## Decision Making

### Routine Changes

For bug fixes, small features, and documentation:
1. Submit PR
2. Receive approval from one maintainer
3. Merge

### Significant Changes

For new features, API changes, or architectural decisions:
1. Open an issue for discussion
2. Allow 7 days for community input
3. Maintainer consensus (or Project Lead decision if contested)
4. Document decision in DECISIONS.md if architectural

### Breaking Changes

For changes that break backward compatibility:
1. RFC document required
2. 14-day comment period
3. Project Lead approval required
4. Clear migration path documented

## Design Philosophy

All contributions should align with Blood's core values:

1. **Memory safety without garbage collection** - Generational references and region-based memory
2. **Algebraic effects for control flow** - Effects instead of exceptions, async, or generators
3. **No hidden costs** - Performance characteristics must be predictable
4. **Gradual complexity** - Simple things should be simple; complex things possible

See [SPECIFICATION.md](docs/spec/SPECIFICATION.md) for detailed design rationale.

## Communication Channels

- **GitHub Issues**: Bug reports, feature requests
- **GitHub Discussions**: Design discussions, questions
- **Pull Requests**: Code contributions

## Release Process

### Versioning

Blood follows [Semantic Versioning](https://semver.org/):
- **Major**: Breaking changes
- **Minor**: New features, backward compatible
- **Patch**: Bug fixes

### Release Cycle

- **Development**: Continuous on `main` branch
- **Release Candidates**: Tagged for testing
- **Stable Releases**: After RC validation

### Release Checklist

- [ ] All tests passing
- [ ] Documentation updated
- [ ] CHANGELOG updated
- [ ] Version numbers updated
- [ ] Release notes written
- [ ] Binaries built and tested

## Conflict Resolution

1. **Technical disputes**: Discuss in issue, seek consensus
2. **No consensus after 7 days**: Project Lead decides
3. **Code of Conduct violations**: Project Lead handles directly
4. **Governance disputes**: Project Lead has final authority

## Amendments

This governance document may be amended by:
1. Proposal via issue or PR
2. 14-day comment period
3. Project Lead approval

## Sustainability

### Bus Factor Mitigation

To reduce single-maintainer risk:
1. Comprehensive documentation (CONTRIBUTING.md, DECISIONS.md)
2. Clear code comments on complex algorithms
3. Automated testing for critical paths
4. Knowledge sharing through contributor mentorship

### Succession Planning

If the Project Lead becomes unavailable:
1. Longest-tenured active maintainer assumes interim leadership
2. Community discussion to select new Project Lead
3. Document transition in this file

## License

Blood is dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

All contributions are assumed to be under the same license unless explicitly stated otherwise.

---

*Last updated: 2026-01-13*
