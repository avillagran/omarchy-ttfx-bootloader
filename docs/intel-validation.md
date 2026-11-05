# Intel validation

The 0.5.1 integration is validated on a physical MacBook Pro with an Intel
Core i5-5257U (Broadwell, x86_64), Omarchy 4.0.6-1, Plymouth 26.134.222-3,
and Rust 1.99.1. It also targets the Omarchy 4.5 installation paths.

## Scope

All compilation and runtime checks run on the Intel machine. The source and
build directories live under `/var/tmp`, on disk. Tests run as an ordinary
user; the module is built and staged into a separate directory. This does not
activate Plymouth, rebuild the host initramfs, test LUKS input at boot, or
establish visual boot acceptance.

The host needed strace and the Rust Clippy component. The isolated offline
check resolves the Rustup toolchain binaries before replacing CARGO_HOME so
mise shims cannot attempt an offline toolchain installation.

## Version and execution path

Upstream 0.4.0 introduced assembly, but 0.5.0 removed it; the latest release
checked for this integration is 0.5.1, commit
`14ed2742596f0056b13e25d14a83ea851adc2294`.
The immutable upstream archive is checked against the vendor manifest, including
all 123 imported files and 29 registry dependencies.

Plymouth calls `Effect::next_frame` and consumes structured cells. It does not
call the upstream `fx` terminal runner. This release therefore does not claim
assembly acceleration for the boot animation. Adopting `fx` requires a
resumable structured-cell interface, deterministic host-owned timing, final
color-band parity, and the existing reset/completion contracts.

## Reproduction

Run `bash tests/verify.sh` from the repository root on the target machine.
The runner requires Rust with Clippy, GCC, Python 3, Node.js, strace, and the
matching Plymouth libraries/headers. It includes:

- Installer static checks, keyboard hook order, menu preservation/idempotence,
  effect selection, simulated UKI/classic initramfs installation and rollback,
  and package staging/failure handling.
- 24 bridge/FFI tests and 40 upstream library tests on x86_64.
- Clippy with warnings denied and vendor provenance verification.
- A locked offline build with an empty Cargo home, linked to the real C ABI.
- strace checks for terminal discovery, subprocesses, sleeps, signal setup,
  and ANSI output.
- Sanitized plugin lifecycle/state/handoff tests, real-engine activation,
  and real Plymouth pixelbuffer checks.
- A complete module build with ABI/export/hardening checks and rootless staging.

Fixture directories are retained for inspection; tests do not recursively
clean them up. Symlink fixtures use targets inside their own temporary directory.

## Validated artifact

The built module is a stripped x86-64 ELF shared object. Its SHA-256 is
`d50689a8c08f01ec1b7438cbb3ff52263f27b9515688fb661fa1738cf6040493`.
The staged copy is byte-identical. Plugin replay passed two complete VHS-tape
loops (470 ticks); Plymouth pixelbuffer readback compared every pixel of a
1280×720 frame. No host boot configuration was changed.
