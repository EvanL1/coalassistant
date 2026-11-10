/**
 * 登录屏 - 使用服务端 HttpOnly 会话.
 *
 * 设计:
 *   - 单页 form, 居中
 *   - 错误提示 inline
 *   - 登录成功后由父级 App 自动切换到主界面
 */
import { useEffect, useState } from "react";
import { LoginRateLimitError, tryLogin } from "./auth";

export function LoginScreen({ notice = null }: { notice?: string | null }) {
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  // 登录被限速 (429) 后剩下的等待秒数; 等待期间按钮不可点并倒计时,
  // 不然用户对着"暂不可用"反复点, 越点锁得越久.
  const [retryIn, setRetryIn] = useState(0);

  useEffect(() => {
    if (retryIn <= 0) return;
    const timer = window.setInterval(() => {
      setRetryIn((value) => Math.max(0, value - 1));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [retryIn > 0]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setSubmitting(true);
    try {
      const ok = await tryLogin(user, pass);
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
    } catch (err) {
      if (err instanceof LoginRateLimitError) {
        if (err.retryAfterSeconds != null) {
          // 提示里的秒数跟着倒计时走 (见下面的 shownError), 数到 0 提示就消失.
          setRetryIn(err.retryAfterSeconds);
        } else {
          setError(err.message || "尝试次数过多, 请稍后再试");
        }
      } else if (err instanceof TypeError) {
        // 网络层错误 (断网/超时), 没有服务端给的原因可显示.
        setError("登录服务暂不可用，请稍后重试");
      } else {
        // 服务端给的 reason (限速以外的失败).
        setError(
          err instanceof Error && err.message
            ? err.message
            : "登录服务暂不可用，请稍后重试",
        );
      }
    } finally {
      setSubmitting(false);
    }
  }

  const shownError = retryIn > 0 ? `尝试次数过多, 请 ${retryIn} 秒后再试` : error;

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
            主焦煤配煤优化 · 数据云端保存
          </p>
        </div>

        {notice && (
          <div
            role="status"
            style={{
              background: "#fef3c7",
              color: "#92400e",
              borderRadius: 8,
              padding: "8px 10px",
              fontSize: 13,
              marginBottom: 12,
            }}
          >
            {notice}
          </div>
        )}

        <form onSubmit={submit}>
          <label className="login-field">
            <span className="login-label">账号</span>
            <input
              autoFocus
              type="text"
              value={user}
              onChange={(e) => setUser(e.target.value)}
              placeholder="请输入账号"
              autoComplete="username"
            />
          </label>

          <label className="login-field">
            <span className="login-label">密码</span>
            <input
              type="password"
              value={pass}
              onChange={(e) => setPass(e.target.value)}
              placeholder="请输入密码"
              autoComplete="current-password"
            />
          </label>

          {shownError && (
            <div
              style={{
                color: "var(--c-danger)",
                fontSize: 12,
                marginBottom: 12,
                paddingLeft: 4,
              }}
            >
              ⚠ {shownError}
            </div>
          )}

          <button
            type="submit"
            className="btn btn-primary"
            disabled={submitting || retryIn > 0}
            style={{
              width: "100%",
              height: 48,
              fontSize: 15,
              fontWeight: 600,
              marginTop: 4,
            }}
          >
            {submitting ? "登录中…" : retryIn > 0 ? `${retryIn} 秒后再试` : "登录"}
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
