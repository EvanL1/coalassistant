/**
 * 登录屏 - 轻量门禁.
 *
 * 设计:
 *   - 单页 form, 居中
 *   - 错误提示 inline
 *   - 登录成功后由父级 App 自动切换到主界面
 */
import { useState } from "react";
import { tryLogin } from "./storage";

export function LoginScreen() {
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [error, setError] = useState("");

  function submit(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    const ok = tryLogin(user, pass);
    if (!ok) {
      setError("账号或密码错误");
      // 摇晃动画提示
      const card = document.getElementById("login-card");
      if (card) {
        card.classList.remove("shake");
        // 触发重排
        void card.offsetWidth;
        card.classList.add("shake");
      }
    }
  }

  return (
    <div
      style={{
        minHeight: "100vh",
        background: "linear-gradient(135deg, #0a5fff, #003fb5)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: "0 24px",
      }}
    >
      <div
        id="login-card"
        style={{
          background: "white",
          borderRadius: 20,
          padding: 28,
          width: "100%",
          maxWidth: 360,
          boxShadow: "0 24px 64px rgba(0,0,0,0.25)",
        }}
      >
        <div style={{ textAlign: "center", marginBottom: 24 }}>
          {/* 煤堆图标 (复用 icon.svg) */}
          <img
            src="./icon.svg"
            alt=""
            width={72}
            height={72}
            style={{ borderRadius: 16, marginBottom: 12 }}
          />
          <h1
            style={{
              margin: 0,
              fontSize: 24,
              fontWeight: 800,
              letterSpacing: "-0.5px",
            }}
          >
            豆哥配煤
          </h1>
          <p
            style={{
              margin: "4px 0 0",
              fontSize: 12,
              color: "var(--c-text-3)",
            }}
          >
            主焦煤配煤优化 · 离线运行
          </p>
        </div>

        <form onSubmit={submit}>
          <label className="login-field">
            <span className="login-label">账号</span>
            <input
              autoFocus
              type="text"
              value={user}
              onChange={(e) => setUser(e.target.value)}
              placeholder="doudou"
              autoComplete="username"
            />
          </label>

          <label className="login-field">
            <span className="login-label">密码</span>
            <input
              type="password"
              value={pass}
              onChange={(e) => setPass(e.target.value)}
              placeholder="••••••"
              autoComplete="current-password"
            />
          </label>

          {error && (
            <div
              style={{
                color: "var(--c-danger)",
                fontSize: 12,
                marginBottom: 12,
                paddingLeft: 4,
              }}
            >
              ⚠ {error}
            </div>
          )}

          <button
            type="submit"
            className="btn btn-primary"
            style={{
              width: "100%",
              height: 48,
              fontSize: 15,
              fontWeight: 600,
              marginTop: 4,
            }}
          >
            登录
          </button>
        </form>

        <p
          style={{
            fontSize: 11,
            color: "var(--c-text-3)",
            textAlign: "center",
            marginTop: 16,
            marginBottom: 0,
          }}
        >
          私人工具 · 仅授权用户访问
        </p>
      </div>
    </div>
  );
}
