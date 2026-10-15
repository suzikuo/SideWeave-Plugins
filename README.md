# SideWeave official plugins

This repository contains the source and release tooling for official SideWeave plugins. Official packages use the same manifest, archive layout, SDK, capabilities, checksum, Minisign signature, and Marketplace catalog contract as community plugins.

The repository is distributed under the MIT License; see `LICENSE`.

## Repository layout

- plugins/<id>/manifest.json is the source of truth for package metadata.
- plugins/<id>/Cargo.toml and src/ contain a native backend when the runtime requires one.
- plugins/<id>/ui/ contains the standalone UI build.
- skins/<id>/manifest.json and its package assets are the source of truth for official skins.
- plugins/ui-sdk/ contains the shared UI bridge used by official plugin UIs.
- tools/build-plugin-packages.ps1 builds package ZIPs without installing them into the host.

Official plugins follow the community plugin contract. Rust backends use `sideweave-plugin-api` for protocol types and `sideweave-plugin-sdk` for host services. Add `sideweave-plugin-ipc` when the plugin runs as a separate process. These crates use the same reviewed SideWeave revision; commit `Cargo.lock` to pin it. Do not depend on host internals.

The notification-lab plugin and ssh-process crate are development/support packages. The package builder includes only manifests whose publisher is sideweave and whose matching Cargo package exists.

## Build a package

Install Rust stable/MSVC and Node.js 22. Then run:

    npm ci
    npm run build:plugins

To build one plugin:

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/build-plugin-packages.ps1 -PluginId time

Archives are written under dist/packages. Import a ZIP through the SideWeave app to exercise the same local package installation flow used by plugin developers. The build tool does not write a trusted-local marker or modify the host's app data.

Skin packages use the same third-party skin manifest and ZIP contract, and are built separately because they contain package assets rather than a Rust backend:

    powershell -NoProfile -ExecutionPolicy Bypass -File tools/build-skin-packages.ps1 -SkinId stellar-abyss

The full publish wizard lists both plugins and skins and writes them to their matching Marketplace collections.

## Package and publish with the wizard

Run the full build, signing, GitHub Release, and Marketplace workflow:

    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish-wizard.ps1

The interactive wizard lets you build locally or publish one or more official plugins or skins. Before upload it verifies the source is committed and pushed to the GitHub default branch. It builds ZIP packages, checks the active SideWeave signing key, writes and verifies Minisign and SHA256 assets, creates or updates `plugin-<id>-v<version>` or `skin-<id>-v<version>` Releases in `suzikuo/SideWeave-Plugins`, updates each Marketplace entry to contain only the current version, and commits and pushes `marketplace.json`. After the catalog update succeeds, it deletes older Releases and their Git tags for the items being published. It lists every old tag and asks for confirmation before publishing and pruning.

The wizard asks for the publisher key password once per run and reuses it for every selected package. Leave it blank if the key has no password.

The publisher key defaults to `../_private/sideweave-publisher.key`, Minisign defaults to `../_private/minisign/minisign.exe`, and the Marketplace checkout defaults to `../marketplace`. Override paths with `-SigningKeyPath`, `-MinisignExe`, and `-MarketplacePath` when needed.

Common non-interactive forms:

    # Build and sign one package locally; uncommitted source changes are allowed.
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish-wizard.ps1 -LocalOnly -Id time

    # Build and sign, then show the upload and old-release cleanup plan without changing anything.
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish-wizard.ps1 -PreviewOnly -Id time

    # Publish an already-built signed package without rebuilding it.
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish-wizard.ps1 -SkipBuild -Id time

    # Publish a package without changing the Marketplace or deleting old Releases.
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish-wizard.ps1 -SkipBuild -SkipMarketplaceUpdate -Id time

The version comes from each item's `manifest.json`; native plugin versions must also match their Cargo package. Bump and commit the applicable version fields before publishing. `-Id all` or a comma-separated ID list selects multiple plugins and skins; batch mode uses each item's manifest version. `-SkipMarketplaceUpdate` is for an intentional release-only operation; it also keeps older Releases and tags.

## SDK dependencies

Rust plugin packages depend on `sideweave-plugin-api` for protocol types and `sideweave-plugin-sdk` for host services; external-process packages also depend on `sideweave-plugin-ipc`. The root Cargo.toml points to the SideWeave Git branch. Commit `Cargo.lock` to pin the reviewed revision, and update the lock only after reviewing and publishing the corresponding host change. Plugin business code must not depend on `src-tauri`, the host runtime crate, or a Tauri `AppHandle`.

## Runtime and trust

Web plugins use web_sandbox. Native plugins use dynamic_library or external_process, declare requires_trust: true, and list requested host capabilities in manifest.json. Capabilities gate host API calls; native code can still access the operating system directly. The host verifies signed Marketplace packages and asks users to trust a native publisher on first install.

## Custom plugin detail pages

Every plugin supplies its own detail UI through `ui.entry`; HTML, Vue, and React bundles can define the page content and styling. By default the host adds a plugin identity header. Set `ui.chrome` to `"plugin"` in `manifest.json` to hide that header and let the UI use the full detail content area. The host retains close and minimize controls. The Rust API equivalent is `PluginUiChrome::Plugin`.
