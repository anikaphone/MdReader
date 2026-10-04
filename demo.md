# MdReader 技术文档与排版演示

欢迎体验 **MdReader**！这是一个专为 Windows 打造的**原生秒启动、极轻量** Markdown 文档阅读器。

---

## 1. 核心设计特色

- **纯原生性能**：冷启动 < 50ms，内存通常仅需 15MB~20MB。
- **零额外依赖**：绿色单文件，免去 WebView2 或 Chromium 的臃肿开销。
- **TOC 目录大纲**：自动解析左侧大纲树，点击任意标题平滑跳转。
- **自动热重载**：外部编辑器保存修改后，窗口实时静默刷新。
- **快捷全文搜索**：随时按 `Ctrl+F` 开启搜索浮层。

> [!NOTE]
> 这是一条 GFM 风格的引用提示块。用于展示 MdReader 对 GitHub Markdown 增强语法的原生排版支持。

> [!TIP]
> 你可以使用快捷键 `Ctrl + 滚轮` 或 `Ctrl + +/-` 随意放大缩小字号排版，按 `Ctrl+0` 快速重置 100%。

---

## 2. 任务清单 (Task Lists)

下面是近期的优化计划列表：

- [x] 原生 eframe/egui 窗口脚手架
- [x] Windows 微软雅黑 / CJK 字体自适应渲染
- [x] CommonMark + GFM AST 解析管线
- [x] 代码语法着色（Syntect）与一键复制代码块
- [x] 自动文件监听（Notify）热重载
- [x] Ctrl+F 页面内快速检索与高亮
- [ ] 导出为 PDF / 纯文本

---

## 3. 代码块语法高亮

代码块支持多种主流语言着色，并且右上角自带一键复制按钮：

```rust
// Rust 示例代码
use std::time::Instant;

fn measure_startup_speed() {
    let start = Instant::now();
    println!("MdReader 极速加载完成！");
    let elapsed = start.elapsed();
    println!("耗时: {:?}", elapsed);
}
```

```python
# Python 示例代码
def calculate_memory_footprint():
    """验证原生 Rust 与 Chromium 内存占用对比"""
    native_rust_ram_mb = 18.5
    webview2_ram_mb = 120.0
    saved_ratio = (webview2_ram_mb - native_rust_ram_mb) / webview2_ram_mb
    print(f"原生模式节省了 {saved_ratio * 100:.1f}% 的内存！")
```

```json
{
  "application": "MdReader",
  "version": "0.1.0",
  "platform": "Windows x64",
  "features": [
    "instant_startup",
    "low_memory",
    "hot_reload",
    "toc_outline"
  ]
}
```

---

## 4. 表格排版 (GFM Tables)

| 特性 | 原生 MdReader | 传统 WebView2 阅读器 |
| :--- | :--- | :--- |
| **冷启动耗时** | **< 50ms (瞬时加载)** | 300ms ~ 800ms |
| **基础内存占用** | **~18 MB** | ~80 MB ~ 150 MB |
| **依赖环境** | 纯静态，零依赖绿色运行 | 需系统已安装 WebView2 Runtime |
| **代码块复制** | 支持右上角快捷复制 | 支持 |
| **目录大纲** | 左侧折叠式 TOC 树 | 取决于前端实现 |

---

## 5. 链接与文本排版

支持*斜体*、**粗体**、~~删除线~~、以及`行内代码 (inline code)`。
还可以点击跳转外部链接：[Rust 官网](https://www.rust-lang.org/) 或在文档内部通过大纲跳转。
