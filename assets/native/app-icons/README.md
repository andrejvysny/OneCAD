# Native packaging icon sources

The fifteen platform PNG variants moved unchanged from the retired
`src-tauri/icons` directory. `master.png` is the original repository icon source.
Keep these as native packaging inputs for supported and future platform bundles.
The current minimal macOS packaging script does not select an application icon;
this cleanup does not change bundle appearance by adding one.
