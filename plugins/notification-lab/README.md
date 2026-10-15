# Notification Lab

本地开发用通知测试插件，可以分别发送 Windows 系统通知、宿主桌面通知和侧边栏气泡通知。

宿主自定义通知由宿主按需创建窗口并统一管理，插件不创建或管理通知窗口。桌面卡片使用 `NotificationService::show_desktop`；侧边栏旁的漫画气泡使用 `NotificationService::show_sidebar`。两种窗口各自复用一个 WebView，消息到期或关闭后窗口会销毁。

Windows 原生通知仍通过 `NotificationService::notify` 发送：

```rust
context.notification().notify("标题", "通知正文")?;
```

插件后端可这样调用：

```rust
use agile_tiles_plugin_sdk::{DesktopNotificationKind, DesktopNotificationOptions};

context.notification().show_desktop(
    &DesktopNotificationOptions::new("标题", "通知正文")
        .with_kind(DesktopNotificationKind::Success)
        .with_duration_ms(6_000),
)?;

context.notification().show_sidebar(
    &DesktopNotificationOptions::new("标题", "可以稍长一些的侧边栏消息正文")
        .with_kind(DesktopNotificationKind::Info)
        .with_duration_ms(6_000),
)?;
```

## 本地安装

关闭 Agile Tiles 后，在仓库根目录运行：

```powershell
npm run notification-lab:dev-install
```

重新启动 Agile Tiles，再从插件列表打开“Notification Lab”。该插件仅用于本地开发，不会加入常规官方插件市场构建。
