# CI 草稿目录

这俩文件**应该在** `.github/workflows/` 里, 但我 (Claude Code 远程会话) 用的
GitHub App 没 `workflows` 权限, 推不上去. 你 (人类 collaborator) 在你 Mac 上
拉这个 PR 后, 一句话搞定:

```bash
git mv ci_drafts/ci.yml          .github/workflows/ci.yml
git mv ci_drafts/flutter-hap.yml .github/workflows/flutter-hap.yml
rmdir ci_drafts
git commit -m "ci: 把 CI 草稿移进 .github/workflows/"
git push
```

提交后 CI 会立刻跑, 你看绿/红反馈给我.

## 这俩文件干啥

### ci.yml (替换现有的)
原版只跑 blend_kit_rs Rust 测试. 新版加两条平行链:

| job | 内容 |
|---|---|
| `blend_kit_rs` | 跟以前一样, cargo test |
| `worker` | **PR #4 加的 44 个 vitest + miniflare D1 测试**, 之前没接 CI 这次接 |
| `flutter` | **PR #5 加的 40 个 Dart 测试 + analyze**, 用 subosito/flutter-action |

3 个 job 并行, 都过才算 CI 绿.

### flutter-hap.yml (新增, 实验性)
鸿蒙 .hap 包构建.
- 仅 `workflow_dispatch` 手动触发 或 `hap-v*` tag, 不挂 push/PR
- `runs-on: macos-latest` (鸿蒙 SDK 只 Mac/Windows 有命令行版)
- timeout 30 min
- 第一次跑大概率红, 给我 log 我据此迭代
- 产物: unsigned .hap, GH artifact 14 天保留

## 跟我永久绕开这个权限

仓库 Settings → Integrations → 找到 Claude / 你的 GH App → 加 `workflows: write` 权限.
以后我能直接推 `.github/workflows/`.

但其实就这两个文件, 用 `git mv` 一次也无所谓.
