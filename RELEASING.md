# Releasing AshLogin

## Release flow

1. Verify the working tree is clean.
2. Run:

   ```bash
   cargo check
   cargo test
   ```

3. Commit and push `main`.
4. Create and push a tag:

   ```bash
   git tag -a v<version> -m "v<version>"
   git push origin main
   git push origin v<version>
   ```

5. Generate the Homebrew formula:

   ```bash
   ./scripts/update-homebrew-formula.sh <version>
   ```

6. Copy `packaging/homebrew-tap/Formula/ashlogin.rb` into `life2you/homebrew-tap/Formula/`.
7. Update the tap README and push the tap repository.

## Notes

- The GitHub repository must be public before the Homebrew formula will work for end users.
- The formula test expects `ashlogin --version` to print the crate version.
