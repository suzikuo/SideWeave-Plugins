# 自定义预览演示插件

用于验证独立悬浮预览页面。预览页面位于 preview.html，支持宿主主题变量同步，并通过 postMessage 请求关闭预览。

## 安装与测试

在 SideWeave 插件中心导入 dist/dev/custom_preview_demo-1.0.0.zip，然后悬停侧边栏里的“自定义预览演示”图标。点击“完成一轮”测试插件自有交互，点击“收起预览”测试关闭消息。

这是一个 web_sandbox 插件，没有 native 后端，也不请求宿主权限。
