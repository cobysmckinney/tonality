# Security policy

## Supported versions

Only the latest stable release and the current nightly get security fixes. tonality updates
itself (Settings, Ctrl+,), so staying current is usually just a restart.

## Reporting a vulnerability

Please don't open a public issue. Report it privately through
[GitHub's security advisories](https://github.com/cobysmckinney/tonality/security/advisories/new)
(**Security → Report a vulnerability**) instead.

Include what you found, how to reproduce it (a sample file helps, if one triggers it) and the
version you saw it in. You'll get a reply within a week. Once a fix is released, the advisory
is published with credit to you unless you'd rather stay anonymous.

Things worth reporting include crafted RAW or sidecar files that crash the app or run code,
weaknesses in the in-app updater or its signature checks, and anything that reads or writes
files outside the library you opened.
