# Contributing to OtterLang VSCode Extension

## Development Setup

### Prerequisites
- Node.js 18+ and npm
- VSCode 1.75.0 or higher
- OtterLang LSP server (for testing language features)

### Setup
```bash
cd vscode-extension
npm install
npm run compile
```

### Development Workflow
1. Make changes to TypeScript source files in `src/`
2. Run `npm run compile` to build
3. Test changes by opening the extension in VSCode's Extension Development Host
4. Package with `vsce package` for testing

## Building & Testing

### Build Extension
```bash
npm run compile
```

### Package Extension
```bash
npx vsce package
```

### Test in VSCode
1. Open VSCode
2. Go to Extensions view
3. Click the "..." menu → "Install from VSIX"
4. Select the generated `.vsix` file

### Run Tests
```bash
npm test
```

## Code Style

- Use TypeScript with strict mode
- Follow VSCode extension API patterns
- Use async/await for asynchronous operations
- Add JSDoc comments for public APIs
- Run `npm run lint` before committing

## Commit Messages

Use clear, descriptive messages following conventional commits:

```
feat: Add support for semantic token highlighting
fix: Resolve parameter hints not showing for methods
docs: Update configuration documentation
refactor: Simplify LSP client initialization
```

## Pull Requests

1. **Keep PRs focused**: One feature or fix per PR
2. **Test thoroughly**: Verify all features work in Extension Development Host
3. **Update documentation**: Keep README.md and this file current
4. **Follow extension guidelines**: Ensure compatibility with VSCode extension marketplace requirements

## Areas for Contribution

### Syntax Highlighting
- Improve TextMate grammar rules
- Add semantic token support
- Enhance syntax coloring for edge cases

### Language Features
- Implement missing LSP features
- Improve IntelliSense suggestions
- Add more code snippets

### Code Editing
- Enhance auto-indentation rules
- Improve auto-closing behavior
- Add smart selection support

### Bug Fixes
- Fix syntax highlighting issues
- Resolve LSP communication problems
- Improve error handling

### Documentation
- Improve README.md
- Add more code examples
- Create video tutorials

## Extension Architecture

### Key Files
- `src/extension.ts` - Main extension entry point
- `src/client.ts` - LSP client implementation
- `syntaxes/otterlang.tmLanguage.json` - TextMate grammar
- `language-configuration.json` - Language configuration

### LSP Integration
The extension communicates with the OtterLang LSP server via:
- Language client initialization
- Document synchronization
- Diagnostics reporting
- Code completion requests

## Testing Your Changes

### Manual Testing
1. Build the extension: `npm run compile`
2. Package: `npx vsce package`
3. Install in VSCode: Extensions → Install from VSIX
4. Test all features with `.ot` files

### Automated Testing
```bash
npm test
```

## Debugging

### LSP Communication
Enable trace logging in VSCode settings:
```json
{
  "otterlang.lsp.trace": "verbose"
}
```

### Extension Logs
View extension output in VSCode's Output panel (select "OtterLang Extension" from dropdown).

## Publishing

### Marketplace Requirements
- Extension must pass VSCode validation
- Include proper metadata in package.json
- No malicious code or excessive permissions
- Follow VSCode extension guidelines

### Release Process
1. Update version in package.json
2. Test thoroughly
3. Package: `vsce package`
4. Submit to marketplace or distribute .vsix file

## Reporting Issues

When reporting bugs, include:
- VSCode version
- Extension version
- OtterLang LSP server version (if applicable)
- Steps to reproduce
- Expected vs actual behavior
- Screenshots if UI-related

## Development Tips

### VSCode Extension API
- Use official VSCode API documentation
- Follow extension development best practices
- Test on multiple platforms when possible

### LSP Protocol
- Understand LSP specification for advanced features
- Use VSCode's built-in LSP debugging tools
- Test with different LSP server versions

### Performance
- Minimize bundle size
- Avoid blocking operations on main thread
- Use web workers for heavy computations if needed

## License

By contributing, you agree that your contributions will be licensed under the BSD 3-Clause License.