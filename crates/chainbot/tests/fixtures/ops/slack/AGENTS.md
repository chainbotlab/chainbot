# Local Rules

## Architecture
- Position: Leaf fixture folder for pass-style encrypted secret files.
- Logic: Holds encrypted-file placeholders mapped from secret references.
- Constraints: Keep filenames aligned with `secret://` lookup semantics (`<name>.gpg`).

## Members
- `webhook.gpg`: Placeholder encrypted file for runtime provider path resolution tests.
