# waft desktop shell

This is the Step 4 Tauri 2 shell. It connects to the existing waft daemon and
starts it when needed. The popover includes the device header, receiving mode,
nearby peer cards, LAN/Internet route badges, and daemon status. Transfers are
implemented in later steps.

## macOS development

Requirements: Rust, Node.js/npm, and Xcode Command Line Tools with the macOS
WebKit development libraries available.

```sh
cd app
npm install
npm run tauri -- dev
```

The app opens a compact popover and adds a waft icon to the system tray/menu
bar. Close the window to hide it; use the tray icon's Open waft action to show
it again. Choose Quit waft from the tray menu to exit the app. The footer
reports whether the existing daemon is connected.

Set `WAFT_DAEMON_PATH` when the `waft` executable is not on `PATH`:

```sh
WAFT_DAEMON_PATH=/path/to/waft npm run tauri -- dev
```

Step 4 is being validated on macOS first. Linux and Windows development and
packaging paths will be completed in their planned later steps.
