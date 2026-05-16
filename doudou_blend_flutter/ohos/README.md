# 鸿蒙 NEXT 接入指南

这个目录留给 **Flutter for OpenHarmony** 生成的 `ohos/` 工程文件用. 第一次
build 前要走以下步骤.

## 1. 装 DevEco Studio + 鸿蒙 SDK

- 去 [华为开发者官网](https://developer.huawei.com/consumer/cn/deveco-studio/)
  下载 DevEco Studio (Windows / Mac), 装鸿蒙 SDK (推荐对应 Mate X5 的 API
  level, 当前 6.1 = API 23).
- 注册 [华为开发者账号](https://developer.huawei.com/), 个人 99 元/年.
- 给 Mate X5 装鸿蒙调试证书 (DevEco 引导).

## 2. 装 Flutter for OpenHarmony fork SDK

官方 Flutter 不支持鸿蒙, 用社区维护的 fork:

```bash
# 拉 fork (镜像比 GitHub 快)
git clone https://gitcode.com/openharmony-sig/flutter_flutter.git
export PATH=$PWD/flutter_flutter/bin:$PATH

# 验证
flutter --version
# 应当包含 "OpenHarmony" 标识
```

## 3. 在本项目 init 鸿蒙工程

```bash
cd /path/to/coalassistant/doudou_blend_flutter
flutter create . --platforms ohos
```

生成的 `ohos/` 目录是 DevEco 标准工程结构, 含:
- `entry/` — 主模块 (类似 Android `app/`)
- `entry/src/main/module.json5` — 应用清单
- `entry/src/main/resources/base/element/string.json` — 多语言文案
- `entry/build-profile.json5` — 构建配置
- `oh-package.json5` — OH 依赖

## 4. 配 pubspec_overrides.yaml

在 `doudou_blend_flutter/` 根目录新建 `pubspec_overrides.yaml`, 把以下
插件指向 OpenHarmony 端口 (因为官方 pub.dev 上的版本只支持
iOS/Android/桌面):

```yaml
dependency_overrides:
  sqflite:
    git:
      url: https://gitcode.com/openharmony-sig/flutter_packages.git
      path: packages/sqflite/sqflite
  path_provider:
    git:
      url: https://gitcode.com/openharmony-sig/flutter_packages.git
      path: packages/path_provider/path_provider
  shared_preferences:
    git:
      url: https://gitcode.com/openharmony-sig/flutter_packages.git
      path: packages/shared_preferences/shared_preferences
```

跑 `flutter pub get`, 检查 `pubspec.lock` 里有 `_ohos` 后缀的实现库.

## 5. 放 Rust 算法库

```bash
cd /path/to/coalassistant
bash doudou_blend_flutter/build_rust.sh ohos arm64

# 把产物放到鸿蒙工程的 libs 目录
mkdir -p doudou_blend_flutter/ohos/entry/libs/arm64-v8a
cp doudou_blend_flutter/build/ohos-arm64/libblend_kit_rs.so \
   doudou_blend_flutter/ohos/entry/libs/arm64-v8a/
```

DevEco 打包 `.hap` 时会自动把 libs/ 下的 .so 一起塞进 `.hap`.

## 6. Build & install

```bash
# 命令行 (DevEco 也能 GUI 跑)
flutter build hap --release
# 产物在 build/ohos/outputs/default/entry-default-signed.hap

# 安装到设备 (Mate X5 USB 接电脑)
hdc install build/ohos/outputs/default/entry-default-signed.hap
```

打开 Mate X5 桌面, 应当能看到 "豆哥配煤" 图标, 点开就用.

---

## 常见问题

**Q: pubspec_overrides 没生效?**
A: 删 `pubspec.lock` + `.dart_tool/`, 重跑 `flutter pub get`.

**Q: hdc 找不到设备?**
A: 在 Mate X5 上: 设置 → 系统和更新 → 开发者选项 → 打开 USB 调试.
首次连接电脑会弹提示, 同意即可.

**Q: 签名错误?**
A: DevEco → File → Project Structure → Signing Configs, 用个人开发者证书.

**Q: SQLite 读写报错?**
A: 检查 `module.json5` 里 `requestPermissions` 是否包含
`ohos.permission.FILE_ACCESS_MANAGER` (写本地数据库需要).
