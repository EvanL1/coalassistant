# 豆哥配煤 agent 环境: 沙箱里预装 `blend` 命令行, 与线上 POST /api/solve 同一套配煤核心.
#
# 构建上下文由 e2b/build.sh 在临时目录里组装, 只含 blend_kit_rs 的源码与数据 ——
# 不能直接用仓库根目录, 否则 1GB+ 的 target/ 会被整个上传.
#
# 单阶段: 装 Rust → 编 blend → 删工具链. master 数据编进二进制, 改数据需重建模板.
# rustup 必须带 --no-modify-path: 否则它往 /root/.bashrc 写 `. $HOME/.cargo/env`,
# 工具链删掉后每个 shell 都会报错, E2B 收尾阶段因此构建失败.
# 源码放 /opt 不放 /tmp: 构建 VM 从缓存层恢复时 /tmp 会被清空, COPY 进去的文件就没了.

FROM e2bdev/code-interpreter:latest

USER root

COPY blend_kit_rs /opt/blend_kit_rs
COPY README.sandbox.md /home/user/README.md

RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential curl ca-certificates \
 && curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path \
 && cd /opt/blend_kit_rs \
 && /root/.cargo/bin/cargo build --release --locked --bin blend \
 && install -m 0755 target/release/blend /usr/local/bin/blend \
 && cd / \
 && rm -rf /opt/blend_kit_rs /root/.cargo /root/.rustup \
 && rm -rf /var/lib/apt/lists/* \
 && blend master > /dev/null \
 && chown user:user /home/user/README.md

WORKDIR /home/user
