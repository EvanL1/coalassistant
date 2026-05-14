# 豆哥配煤 (Doudou Blend)

主焦煤配煤优化 - 移动端 APP. 把 8 项煤质指标 + 价格约束求解为最优配比.

## 仓库结构

```
.
├── blend_kit_rs/        Rust 核心算法 (Clarabel LP 求解器)
│   ├── src/             model / optimizer / predict / seed
│   ├── data/            煤种 master 数据库 (JSON + CSV)
│   ├── docs/            JSON API Schema
│   └── examples/        demo / master_demo
├── doudou_blend/        Tauri 2.0 移动端 APP
│   ├── src/             React TypeScript 前端
│   └── src-tauri/       Rust 后端 + SQLite 本地数据
├── mockup/              HTML 设计稿 (6 屏视觉)
└── .github/workflows/   CI + Release 自动构建
```

## 核心特性

- **8 项指标**: 硫/灰/挥发/粘结/胶质/岩相/焦炭强度/水分
- **价格分离**: FOB (出厂价) + FRT (运费), CIF (到厂价) 自动计算
- **LP 求解**: Clarabel, < 1ms / 次
- **三视图输出**: 成本结构 / 实物订单 / 指标体检
- **binding 检测**: 自动识别"顶格"约束, 反推谈判方向
- **CSR 预测**: 可选历史回归校准
- **离线优先**: SQLite 本地存储, 73 种煤 master 预置

## 本地开发

```bash
# 跑 Rust 核心 demo
cd blend_kit_rs
cargo run --release --example master_demo

# 跑测试
cargo test --release

# 启动 Tauri 桌面 dev (需 Node 22+)
cd ../doudou_blend
npm install
npm run tauri dev
```

## 打包发布

打 tag 触发 GitHub Actions 自动构建:

```bash
git tag v0.1.0
git push origin v0.1.0
```

产出: Android APK + macOS DMG 上传到 GitHub Releases (草稿).

## 设计哲学

- **每字段一个来源**: 化验/经验/采购/物流分别负责
- **派生量用函数, 不用字段**: CIF = FOB + FRT 不存储
- **不可变快照**: LP 输入是 immutable, 可重现可审计
- **三视图后处理**: 财务/采购/质检三类用户各自的关注点
- **draft 状态机制**: 数据有疑点的煤不静默使用, 让用户先确认

## License

MIT (待定).
