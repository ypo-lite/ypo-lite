# Contributing to Ypo-Lite

Thank you for wanting to help. Ypo-Lite is a set of small Windows tray tools, and
the aim is to keep them small: *low memory, fast, and they do just what they need
to do.* That shapes what is accepted, so please read this page before you start
on anything large.

## Ways to help

- **Report a bug.** Open an issue with your Windows version, the tool and its
  version (`About` in the tray menu), what you did, what you expected and what
  happened. A copy of the relevant part of your `y-snap.xml` or `y-pop.xml`
  helps a lot. For a window that is not handled properly, give its program name
  and, if you can, its window class.
- **Suggest a feature.** Open an issue first and say what you are trying to do,
  not only what you would like added. A feature that needs a runtime, a
  background service or a lot of memory for a small gain will probably be
  declined; a small one that fits the existing shape usually will not.
- **Improve the documentation.** Every tool has a user manual, a configuration
  reference and development notes. Fixes to them are always welcome.
- **Send a pull request.** For anything beyond a small fix, open an issue first
  so we can agree on the approach before you spend time on it.

## Getting set up

You need Windows 10 or 11, [Rust](https://rustup.rs/) 1.95 or later with the MSVC
toolchain, and the Windows SDK (for its resource compiler). Ypo-Lite is
Windows-only.

```sh
git clone https://github.com/ypo-lite/ypo-lite
cd ypo-lite
cargo check --workspace --all-targets
cargo test --workspace
cargo run -p y-snap          # debug run of one tool
```

Before you push, both of these must pass without warnings. Clippy warnings count
as build failures:

```sh
cargo clippy --workspace --all-targets
cargo test --workspace
```

If `cargo check` reports a suspiciously instant "Finished" after you edited a
file, `touch` the changed `.rs` files and run it again.

## Where things are

| Path | What it is |
| --- | --- |
| `core` (`y-core`) | shared Win32 plumbing: message window, tray icon, menus, hotkeys, overlays, `.ini` and XML configuration |
| `y-snap`, `y-pop` | the tools. Each has a `docs/development.md` explaining how its code is organised |
| `y-snap-config`, `y-pop-config` | the WinUI 3 settings apps |
| `ui-kit` | the toolkit shared by the settings apps. Read [`ui-kit/docs/ui-toolkit.md`](ui-kit/docs/ui-toolkit.md) before touching a screen |
| `skeleton` | a self-contained reference application. It is **not** a workspace member and must keep building on its own |

The [main README](README.md#repository-layout) has the full layout, and
[`skeleton/README.md`](skeleton/README.md) explains the Win32 reasoning behind
`core`.

## House rules for the code

These come from how the tools have to behave, so a pull request that breaks one
will be asked to change.

- **`unsafe` and Win32 plumbing belong in `core`**, or in a crate's own thin
  wrapper module (for example `y-snap/src/mouse`). Application code stays a plain
  `match` on messages and typed actions.
- **Low-level hooks must be fast.** Windows silently drops a `WH_KEYBOARD_LL` or
  `WH_MOUSE_LL` hook that takes longer than 300 ms. A hook procedure must not
  allocate, take a lock, create windows, do GDI work, or make a call that blocks
  on another process (`SendMessage`, `WM_NCHITTEST` and the like). Post a private
  `WM_APP`-based message to the owner window and do the work in the normal
  message loop. State used by a hook lives in a `thread_local!`, never a `Mutex`.
- **Add Win32 APIs in one place.** The `windows` crate's feature list is set once,
  in `[workspace.dependencies.windows]` of the root `Cargo.toml`. Do not add
  features in a per-crate `Cargo.toml`.
- **Keep dependencies out.** Reach for the standard library and the existing
  `core` modules first. XML is parsed by hand in `core/src/xml.rs` on purpose.
  Any new dependency needs a reason in the pull request.
- **Overlays are drawn in a fixed order**: fill the panel, draw the text, round
  the corners. GDI text zeroes alpha where it touches, so the corner cut comes
  last.
- **Window targeting errs toward refusing.** `y-snap/src/target.rs` filters out
  shell and transient windows by class name. When you find a window that should
  not be touched, add it to the `BANISHED` list.
- **Follow the configuration severity split.** A malformed XML document aborts
  the load with a line number. Anything recognised but wrong (unknown action, bad
  key combination, unknown attribute) is a non-fatal warning and the rest of the
  file still loads.
- **Say when you simplify.** If you deliberately do less than the GUIPro original
  or than an obvious design would, note it in the doc comment or the README at
  that spot.
- **The version is set in one place**: `[workspace.package] version` in the root
  `Cargo.toml`. Do not edit versions by hand; maintainers use
  `scripts/bump-version.ps1`.

### Ported features

Ypo-Lite rewrites the tools of [GUIPro](http://glatigny.github.io/guipro/). When
you port a feature, treat the original as prior art, not a specification:

- no DLL plugin system: every module is built in;
- configuration formats are new and are not compatible with the old ones;
- do not copy a shortcut from the original without checking that it is safe under
  a low-level hook.

## Documentation

Every user-visible change needs its documentation updated in the same pull
request: the tool's **user manual**, its **configuration reference** (every
setting of the XML file), and the settings app's manual when a screen changes.
New screenshots go in the tool's `docs/images` folder. Write for someone who has
never seen the tool.

## Commits and pull requests

- **Sign your commits.** The `main` branch requires verified signatures, and an
  unsigned commit cannot be merged. [GitHub's guide](https://docs.github.com/authentication/managing-commit-signature-verification)
  shows how to set up GPG, SSH or S/MIME signing.
- **Keep a pull request to one thing.** It is easier to review and to revert.
- **Write a clear commit message** that says what changed and why. The history
  uses short prefixes such as `[feat]`, `[fix]` and `[doc]`, followed by the tool
  in backticks when one is concerned, for example `` [feat] `y-snap` improve
  monitor support ``.
- **Open the pull request against `main`.** It is squash-merged, so the history
  stays linear and your commits are combined into one. Give the pull request a
  title and description that work as the final commit message.
- **Review.** Each pull request needs one approving review, and every discussion
  thread resolved. Changes under `.github/` also need approval from the core team.
- **Say how you tested it.** Include the Windows version and, for anything with a
  visible effect, what you did and saw. A screenshot or a short GIF is welcome.
  Hooks, overlays, tray icons and multi-monitor behaviour are hard to unit test;
  say plainly what you verified by hand and what you did not.
- **Add tests where the logic allows.** Pure logic, such as parsing, state
  machines and geometry, should have a unit test. Run a single one with
  `cargo test -p y-snap config::tests::the_default_configuration_is_valid`.

## Security

Please do not open a public issue for a security problem. Use GitHub's
[private vulnerability reporting](https://github.com/ypo-lite/ypo-lite/security/advisories/new)
on the repository's *Security* tab.

## License

Ypo-Lite is [GPL-3.0-or-later](LICENSE). By contributing, you agree that your
contribution is licensed under the same terms.
