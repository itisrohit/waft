# waft desktop shell

This is the Step 1 Tauri 2 shell. It is currently a placeholder and does not
connect to the daemon or perform discovery or transfers.

## macOS development

Requirements: Rust, Node.js/npm, and Xcode Command Line Tools with the macOS
WebKit development libraries available.

```sh
cd app
npm install
npm run tauri dev
```

The app opens a small placeholder window and adds a waft icon to the system
tray/menu bar. Close the window to hide it; use the tray icon's Open waft action
to show it again. Choose Quit waft from the tray menu to exit the app.

Step 1 is being validated on macOS first. Linux and Windows development and
packaging paths will be completed in their planned later steps.
