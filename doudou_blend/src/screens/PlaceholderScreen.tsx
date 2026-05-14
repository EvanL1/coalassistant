/** 简单占位屏: 用于合同/历史/我的, 后续实装. */
import type { JSX } from "react";

interface Props {
  title: string;
  subtitle?: string;
  hint?: string;
  icon?: JSX.Element;
}

export function PlaceholderScreen({ title, subtitle, hint, icon }: Props) {
  return (
    <>
      <div className="page-header">
        <h1 className="page-title">{title}</h1>
      </div>
      <div className="placeholder-screen">
        {icon && <div style={{ marginBottom: 16 }}>{icon}</div>}
        <h2>{subtitle || `${title}功能开发中`}</h2>
        {hint && <p style={{ marginTop: 8, fontSize: 13 }}>{hint}</p>}
      </div>
    </>
  );
}
