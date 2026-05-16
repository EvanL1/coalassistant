# 豆哥配煤 - Flutter 鸿蒙 NEXT 版

Flutter 实现, 主要目标平台是 **HarmonyOS NEXT** (鸿蒙 5+/6+, 纯血鸿蒙, 不
含 Android 兼容层). 数据全部存本地 SQLite, 算法走 Rust FFI, 不依赖任何
后端服务.

跟 monorepo 里 `doudou_blend/` (React+Tauri+CF Worker 版) **并行维护**:
- 业务模型/SQLite schema 一一对齐, 方便以后 JSON 互导
- 共用 `blend_kit_rs` 做配煤求解
- UI / 状态管理两套独立

---

## 架构

```
doudou_blend_flutter/
├── pubspec.yaml          # deps (provider, sqflite, ffi, go_router 等)
├── lib/
│   ├── main.dart         # 入口
│   ├── app.dart          # MaterialApp + GoRouter
│   ├── models/           # Dart 数据类 (跟 types.ts 对齐)
│   ├── data/             # Repository: SQLite CRUD
│   ├── services/         # coal_master loader, blend solver (FFI + fallback)
│   ├── state/            # Provider 状态
│   └── ui/
│       ├── theme.dart    # Material 3, 蓝主色
│       ├── shell.dart    # 5 tab 底栏
│       ├── screens/      # 6 屏 (今日/客户/报价/合同/我 + 煤池子屏)
│       ├── dialogs/      # 模态弹窗
│       └── widgets/      # 共享 UI 组件
├── assets/
│   └── coal_master.json  # 73+ 煤种 master 数据 (跟 web 版同源)
├── native/               # blend_kit_rs cdylib 构建 (各平台)
├── ohos/                 # 鸿蒙特定配置 (DevEco 接入)
└── test/                 # 业务/数据层单测
```

---

## 开发流程

### 桌面 / Web 验证 (在 Mac/Linux/Windows 任意机)

```bash
cd doudou_blend_flutter
flutter pub get
flutter run -d chrome        # 跑在浏览器, 调 UI 最快
flutter test                  # 单测全跑
```

桌面端走 `sqflite_common_ffi`, 数据存 `~/Documents/doudou_blend.db`.
Rust FFI 默认 fallback 到 Dart 均分实现 (跑 LP 必须编 cdylib, 见 `native/README.md`).

### 鸿蒙 NEXT 真机

**前置**: 装 DevEco Studio (Windows/Mac), 鸿蒙开发者账号, Mate X5 开启
调试模式.

1. 装 Flutter for OpenHarmony fork SDK:
   ```bash
   git clone https://gitcode.com/openharmony-sig/flutter_flutter.git
   export PATH=$PWD/flutter_flutter/bin:$PATH
   flutter --version       # 确认是 OH fork
   ```
2. 在项目里 init 鸿蒙工程目录:
   ```bash
   cd doudou_blend_flutter
   flutter create . --platforms ohos
   # 会生成 ohos/ 目录, 含 entry / 签名配置 / DevEco 工程
   ```
3. 配 `pubspec_overrides.yaml` 把插件指向 OH 端口 (见 `ohos/README.md`):
   ```yaml
   dependency_overrides:
     sqflite:
       git: https://gitcode.com/openharmony-sig/flutter_packages.git
       path: packages/sqflite/sqflite
     path_provider:
       git: https://gitcode.com/openharmony-sig/flutter_packages.git
       path: packages/path_provider/path_provider
     shared_preferences:
       git: https://gitcode.com/openharmony-sig/flutter_packages.git
       path: packages/shared_preferences/shared_preferences
   ```
4. 编 Rust cdylib 给鸿蒙 (见 `native/README.md`):
   ```bash
   bash build_rust.sh ohos arm64
   cp build/ohos-arm64/libblend_kit_rs.so ohos/entry/libs/arm64-v8a/
   ```
5. 用 DevEco Studio 打开 `ohos/` 目录, run 到 Mate X5.

---

## 测试

```bash
flutter test           # 全跑
flutter test test/database_test.dart   # 单文件
```

测试用 `sqflite_common_ffi` 跑内存 SQLite, 不依赖真机.

---

## 数据互导

跟 `doudou_blend` web 版的 D1 schema 完全一致, 后续可加 JSON 导入导出:
- 用户在 web 版导出 JSON
- 微信发给豆哥
- 豆哥在 Flutter app 里导入

字段映射零损耗.
