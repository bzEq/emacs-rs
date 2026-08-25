# emacs-rs

一个使用 Rust 编写的 Emacs-like 文本编辑器:rope 数据结构带来的大文件性能、
Emacs 的键绑定与命令系统、LuaJIT 作为扩展语言。

## 特性

- **Rope Buffer**:基于 `ropey` 的 buffer,编辑 O(log n);100MB 日志文件约 70ms
  打开(约 1.5GB/s),1GB 文件可流畅编辑
- **Emacs 编辑体验**
  - 经典键位:移动/编辑/kill-ring/undo/mark、`C-x` 前缀键、Esc 作为 Meta
  - 命令系统:`M-x` 可运行任意命令,自动补全(输入即补全公共前缀,TAB 循环)
  - 增量搜索:`C-s` / `C-r`,大小写不敏感、越界回绕、`C-g` 中止
  - Undo(带边界)、kill ring(连续 kill 累积)、prefix argument(`C-u`/`C-3`)
  - CRLF 文件按 Emacs 语义处理(`\r\n` 视为单个换行)
- **窗口系统**:`C-x 2/3` 分割、`C-x 0/1` 删除、`C-x o` 切换,每窗口独立 point
  与滚动位置
- **Dired**:`C-x d` 目录浏览器——列表、标记(m/u/U)、删除(D)、重命名(R)、
  复制(C)、建目录(+)、子目录导航;`find-file` 或命令行参数为目录时自动打开
- **语法高亮**:tree-sitter(内置 Rust / Lua),按节点类型着色,带解析上限与
  重解析冷却,大文件不卡顿
- **自动缩进**:`RET` 智能缩进(`{` 缩进、`}`/`end` 回退)、`TAB` 重排当前行、
  `C-j` = `electric-newline-and-maybe-indent`(注释/字符串内不缩进)、行首
  Backspace 删除一个缩进单位
- **Major / Minor mode**:按扩展名选 major mode;`line-numbers` 等 minor mode
  可切换;mode 可带本地 keymap(modeline 显示 lighter)
- **LuaJIT 扩展**:内置 `mlua` + vendored LuaJIT,无需系统依赖;`init.lua`
  可定义命令、绑定键位、定义 major/minor mode、注册 hook

## 构建与运行

依赖:稳定的 Rust 工具链(项目使用 edition 2021,开发环境为 nightly 1.92,
stable 亦可)与 C 编译器(LuaJIT vendored 编译需要)。

```sh
cargo build --release
./target/release/em [--init <init.lua>] [FILE]
```

- `FILE` 为文件时打开该文件;为目录时用 dired 打开
- `--init` 指定 init 文件,默认 `~/.config/emacs-rs/init.lua`
  (尊重 `XDG_CONFIG_HOME`)

## 配置(init.lua)

参见 [`examples/init.lua`](examples/init.lua) 中的完整注释示例。核心 API:

```lua
-- 命令与键位
emacs.define_command("my-cmd", function(prefix) emacs.insert("x") end)
emacs.bind("C-c x", "my-cmd")            -- 全局绑定(覆盖默认)
emacs.local_set_key("C-c y", "my-cmd")   -- 当前 buffer 本地绑定

-- major / minor mode
emacs.define_major_mode("txt-mode", {
  indent = 2,
  language = "lua",                      -- 可选,启用高亮
  keymap = { ["C-c h"] = "my-cmd" },
})
emacs.define_minor_mode("my-extra", {
  lighter = "XX",
  keymap = { ["C-c e"] = "my-cmd" },
})

-- buffer 操作(均作用于当前 buffer)
emacs.insert("text"); emacs.point(); emacs.set_point(n)
emacs.buffer_string(); emacs.save_buffer(); emacs.execute("command")

-- hook
emacs.add_hook("before_save", function() emacs.message("saving...") end)
```

键位语法:`C-x C-f`、`M-f`、`C-M-a`、`RET`、`TAB`、`DEL`、`SPC`、`<left>`、`<f1>`。

## 常用键位

| 键 | 命令 | 键 | 命令 |
|---|---|---|---|
| `C-f/b/n/p` | 前后字符/上下行 | `C-x C-f` | 打开文件 |
| `M-f/M-b` | 前后单词 | `C-x C-s` | 保存 |
| `C-a/C-e` | 行首/行尾 | `C-x d` | dired 目录浏览 |
| `C-k/C-w/M-w` | kill 行/区域/复制 | `C-x b/k` | 切换/关闭 buffer |
| `C-y/M-y` | yank / yank-pop | `C-x 2/3/0/1/o` | 窗口操作 |
| `C-/` `C-_` `C-x u` | undo | `C-s/C-r` | 增量搜索 |
| `C-g` | 取消 | `M-x` | 执行命令(带补全) |
| `C-u/C-3/M--` | 数字参数 | `C-h k/b` | 查看键位 |

## 架构

```
crates/
  core/   # 纯逻辑:rope buffer、undo、kill ring、keymap、命令系统、
          # window tree、isearch、dired、缩进、tree-sitter 高亮
  lua/    # mlua + LuaJIT:emacs 模块(命令/键位/mode/hook API)
  ui/     # ratatui 渲染:窗口树、modeline、echo area、补全预览
  app/    # em 二进制:事件循环(read -> execute -> render)、CLI
```

## 测试

```sh
cargo test
```

- `crates/core` 单元测试:buffer 语义(goal column、CRLF)、undo、kill ring、
  keymap、窗口树、isearch、缩进、语法高亮、dired
- `crates/app/tests` PTY 集成测试:在真实伪终端中启动 `em` 二进制,模拟按键、
  重建屏幕并断言(编辑、窗口、搜索、高亮、mode、补全、dired、CLI)

## License

[Apache License 2.0](LICENSE)
