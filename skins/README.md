# Official skins

Official skin packages live here and use the same root `manifest.json`, ZIP layout, publisher identity, signature, checksum, Marketplace entry, and GitHub Release workflow as other plugins. The Marketplace stores skins in its `skins` collection and skin packages declare `type: "skin"`.

To create a skin, copy one of the directories here, give the copy a unique directory and manifest `id`, and update its version and metadata. A skin must provide either `skin.css` or a complete `skinUi` with sidebar, detail, control, and preview entry points. Skin assets stay package-relative.

Build one package with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build-skin-packages.ps1 -SkinId my-skin
```

Build and sign a local release with the shared publisher key:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish-wizard.ps1 -LocalOnly -Id my-skin
```

The source directory is the single publishing source of truth. The SideWeave host contains only its default skin; Pixel and other official skins are installed as standalone packages from the Marketplace.
