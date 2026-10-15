# SideWeave official plugins

This repository contains the source and release tooling for official SideWeave plugins. Official packages use the same manifest, archive layout, SDK, capabilities, checksum, Minisign signature, and Marketplace catalog contract as community plugins.

The repository is distributed under the MIT License; see `LICENSE`.

## Repository layout

- plugins/<id>/manifest.json is the source of truth for package metadata.
- plugins/<id>/Cargo.toml and src/ contain a native backend when the runtime requires one.
- plugins/<id>/ui/ contains the standalone UI build.
- plugins/ui-sdk/ contains the shared UI bridge used by official plugin UIs.
- tools/build-plugin-packages.ps1 builds package ZIPs without installing them into the host.

Official plugins follow the community plugin contract. Rust backends use `agile-tiles-plugin-sdk`, which re-exports API types, and add `agile-tiles-plugin-ipc` only when they run as a separate process. The workspace references these crates from SideWeave; commit `Cargo.lock` to pin the reviewed SDK revision. Do not add a direct API crate dependency or depend on host internals.

The notification-lab plugin and ssh-process crate are development/support packages. The package builder includes only manifests whose publisher is sideweave and whose matching Cargo package exists.

## Build a package

Install Rust stable/MSVC and Node.js 22. Then run:

    npm ci
    npm run build:plugins

To build one plugin:

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/build-plugin-packages.ps1 -PluginId time

Archives are written under dist/packages. Import a ZIP through the SideWeave app to exercise the same local package installation flow used by plugin developers. The build tool does not write a trusted-local marker or modify the host's app data.

## Prepare a signed release

Run from this repository:

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/release-package-local.ps1 -Id time

The release tool reads the package version from the source manifest, checks that the manifest references the active SideWeave publisher key, builds the package, and writes a ZIP, .minisig, and .sha256 file under dist/local-release/plugin-<id>-v<version>.

Create a GitHub Release in suzikuo/SideWeave-Plugins using that tag and upload all three assets. Then update the exact asset URLs and repository field in suzikuo/SideWeave-Marketplace/marketplace.json, and run the Marketplace validator. The plugin repository does not publish catalog changes automatically.

## SDK dependencies

Rust plugin packages depend on `agile-tiles-plugin-sdk`, which re-exports the API types; external-process packages also depend on `agile-tiles-plugin-ipc`. The root Cargo.toml points to the SideWeave SDK Git branch. Commit `Cargo.lock` to pin the reviewed revision, and update the lock only after reviewing and publishing the corresponding host change. Plugin business code must not depend on `src-tauri`, the host runtime crate, or a Tauri `AppHandle`.

## Runtime and trust

Web plugins use web_sandbox. Native plugins use dynamic_library or external_process, declare requires_trust: true, and list requested host capabilities in manifest.json. Capabilities gate host API calls; native code can still access the operating system directly. The host verifies signed Marketplace packages and asks users to trust a native publisher on first install.
