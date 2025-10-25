# Contributing to rustvncserver

Thank you for your interest in contributing to rustvncserver! We welcome contributions from the community.

## Code of Conduct

This project adheres to a Code of Conduct that all contributors are expected to follow. Please read [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) before contributing.

## How to Contribute

### Reporting Bugs

If you find a bug, please create an issue with:
- A clear, descriptive title
- Steps to reproduce the issue
- Expected behavior vs actual behavior
- Your environment (OS, Rust version, rustvncserver version)
- Any relevant logs or error messages

### Suggesting Features

We welcome feature suggestions! Please create an issue with:
- A clear description of the feature
- Why you think it would be useful
- Any implementation ideas you have

### Pull Requests

1. **Fork the repository** and create your branch from `main`
2. **Write code** following our style guidelines (see below)
3. **Add tests** for any new functionality
4. **Update documentation** if you've changed APIs
5. **Run tests** to ensure everything passes
6. **Submit a pull request** with a clear description

## Development Setup

### Prerequisites

- Rust 1.90 or later
- `libjpeg-turbo` (optional, for turbojpeg feature)

### Architecture

rustvncserver uses the separate [rfb-encodings](https://github.com/dustinmcafee/rfb-encodings) library for all encoding implementations. This modular design allows:
- Encoding code to be reused across VNC servers, proxies, and recording tools
- Independent versioning and publishing of encoding implementations
- Better separation of concerns (protocol vs encoding)

### Building

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/rustvncserver.git
cd rustvncserver

# Build
cargo build

# Run tests
cargo test

# Build with all features
cargo build --all-features
```

### Running Examples

```bash
cargo run --example simple_server
cargo run --example headless_server
```

## Style Guidelines

### Code Style

- Follow standard Rust formatting: `cargo fmt`
- Pass clippy lints: `cargo clippy -- -D warnings`
- Write idiomatic Rust code
- Keep functions focused and reasonably sized
- Use descriptive variable names

### Documentation

- Add doc comments (`///`) for all public items
- Include examples in doc comments where helpful
- Keep documentation up to date with code changes
- Write clear commit messages

### Commit Messages

Follow the [Conventional Commits](https://www.conventionalcommits.org/) format:

```
type(scope): subject

body (optional)

footer (optional)
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `perf`: Performance improvements
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

**Examples:**
```
feat(encoding): add support for ZSTD compression
fix(auth): handle empty password correctly
docs(readme): update installation instructions
```

## Testing

### Writing Tests

- Write unit tests for new functions
- Add integration tests for new features
- Ensure tests are deterministic
- Use descriptive test names

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run with all features
cargo test --all-features
```

## Adding New Encodings

If you're adding a new VNC encoding:

1. Contribute to the [rfb-encodings](https://github.com/dustinmcafee/rfb-encodings) library instead
2. Create a new file in the rfb-encodings `src/` directory
3. Implement the encoding following RFC 6143 specification
4. Add comprehensive tests
5. Update the rfb-encodings README.md with the new encoding
6. Update rustvncserver to use the new encoding from rfb-encodings
7. Add example demonstrating the encoding in rustvncserver

**Note:** All encoding implementations are now maintained in the separate rfb-encodings crate for reusability across VNC servers and other tools.

## Performance Considerations

- Profile code changes for performance impact
- Avoid unnecessary allocations
- Use zero-copy techniques where possible
- Consider async I/O implications
- Document any performance trade-offs

## Documentation

### API Documentation

Generate and review documentation:

```bash
cargo doc --open --all-features
```

Ensure all public items have:
- Summary description
- Parameter explanations
- Return value description
- Example code (where appropriate)
- Links to related items

### README Updates

If your changes affect:
- Features
- API
- Examples
- Installation

Please update README.md accordingly.

## Release Process

Maintainers follow this release process:

1. Update CHANGELOG.md
2. Bump version in Cargo.toml
3. Update version references in README.md
4. Create git tag: `git tag -a v1.x.x -m "Release v1.x.x"`
5. Push tag: `git push origin v1.x.x`
6. Publish to crates.io: `cargo publish`

## Getting Help

- Create an issue for questions
- Join discussions on GitHub
- Check existing issues and PRs
- Read the [TECHNICAL.md](TECHNICAL.md) for implementation details

## License

By contributing, you agree that your contributions will be licensed under the Apache-2.0 License.

## Recognition

Contributors are recognized in:
- Git commit history
- Release notes
- CHANGELOG.md

Thank you for contributing to rustvncserver! 🦀
