# Optional sudo / 1Password threat model

Kettle does not need privileges for discovery. Homebrew remains the only component that mutates packages. The optional `kettle-askpass` helper only supplies an existing macOS account password when Homebrew invokes `sudo -A` during operations such as cask installation.

## Assets and data flow

The protected secret is the user's macOS login password stored in a 1Password item. Kettle stores only an `op://` reference in `~/.config/kettle/sudo-secret`.

1. Homebrew invokes `sudo -A` because Kettle supplied an absolute `SUDO_ASKPASS` path.
2. `sudo` executes the bundled `kettle-askpass` helper.
3. The helper validates the reference file, resolves `op` from a fixed Homebrew location, and runs `op read --no-newline <reference>` directly without a shell.
4. The 1Password CLI retrieves the password according to the user's current 1Password authorization and policy.
5. The helper writes only the password bytes to stdout, flushes the pipe, and overwrites its output buffer. `sudo` consumes those bytes.

The password can therefore exist in the 1Password process, the helper's heap, the kernel pipe, and `sudo` memory. It is never intentionally placed in application logs, environment variables, temporary files, or command arguments. The `op://` reference, which names a vault/item/field but is not the password, is present in the `op` argument vector.

## Trust boundaries and controls

- Kettle accepts an askpass helper only when it is a regular executable resolving inside the running application's executable directory, owned like the application executable, and neither the file nor directory is group/world writable.
- The helper accepts only a regular reference file owned by the home-directory owner with no group/world permissions. Its parent must be owned by the same user and not group/world writable.
- References are bounded, control-character-free `op://` values containing a vault, item, and field, with an optional section.
- `op` is resolved only from `/opt/homebrew/bin/op` or `/usr/local/bin/op`; the canonical executable and every parent component must be owned by root or the invoking user and must not be group/world writable. `PATH` is not trusted.
- No shell parses the command. The helper has no third-party Rust dependencies and forbids unsafe Rust through workspace lint policy.
- The helper's sole stdout write is the password after a successful `op` exit. Every validation, launch, authorization, and read failure exits non-zero without writing stdout. Diagnostics go to stderr and never include password bytes.

## 1Password authorization

Kettle does not bypass or strengthen 1Password authorization. Whether `op read` requires biometric approval, an unlocked desktop app, or an active CLI session is controlled by the user's 1Password configuration and may change outside Kettle. Successful retrieval proves only that 1Password authorized that read under its current policy. Kettle does not cache the retrieved password.

## Same-user attacker

File modes do not protect against a malicious process already running as the same user. Such a process may execute the helper, alter same-user-owned configuration, observe user-controlled files, interact with an already-authorized 1Password session, or request sudo itself. Platform process protections can make cross-process memory access harder, but Kettle does not treat them as a security boundary. The helper reduces accidental exposure and PATH/config substitution; it cannot make a compromised login session trustworthy.

## Why not Authorization Services

Authorization Services can grant named rights and support a separately installed privileged helper. It does not transparently satisfy Homebrew's internal `sudo` prompts. Making Kettle own a privileged helper for Homebrew's changing set of cask filesystem and installer operations would either reimplement Homebrew mutations or expose a broad root command surface. Both are worse than leaving Homebrew authoritative and using the user's existing sudo policy. Kettle therefore keeps askpass opt-in rather than installing a privileged service.

## Residual risk and disabling

The password necessarily exists briefly in multiple processes and a pipe. Memory overwriting is best effort: compiler, allocator, kernel, and subprocess copies are outside Kettle's control. A compromised user session or authorized 1Password session remains able to retrieve the credential.

Delete `~/.config/kettle/sudo-secret` to disable the feature. Kettle then omits `SUDO_ASKPASS`, and Homebrew/sudo use their normal interactive behavior.
