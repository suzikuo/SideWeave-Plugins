# SideWeave native plugin development template

This sample is part of the SideWeave-Plugins workspace. It uses the same native SDK and package manifest contract as official and community plugins. Its placeholder publisher keeps it out of the official package list.

Copy this directory to plugins/<id> in the SideWeave-Plugins checkout. Update the Cargo package name, manifest ID, version, publisher, key_id, capabilities, and UI metadata before packaging. Replace the sample code and icon as well.

Cargo.toml inherits the SDK dependency from the plugin repository workspace. The SDK re-exports the API types, so plugin code does not add a separate API crate dependency. External-process plugins can add the IPC crate. Plugin code must not depend on the host src-tauri project or Tauri AppHandle. Use PluginContext for host storage, files, windows, and system operations.

Declare requested host API capabilities in manifest.json. For a native package set type to native and requires_trust to true. Native code can access the operating system directly; capabilities only gate host API calls.

The UI is a separate Vite/Vue bundle under ui/. It must not import from the host src directory and should use relative asset URLs. The package builder compiles the UI and places the manifest, backend DLL, icon, and generated UI files into one ZIP.

For web plugins, use the separate suzikuo/SideWeave-plugin-template repository. Repository roles, package rules, and release steps are documented in SideWeave/docs/PLUGIN_REPOSITORY.md.
