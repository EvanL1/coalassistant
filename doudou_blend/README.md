# 豆哥配煤 · 前端

React 19 + TypeScript + Vite，对接 `blend_kit_server` 的同源 `/api` 接口。

## 开发

```bash
npm install
npm run dev     # Vite :1420, /api 代理到本地 blend_kit_server (:3000)
```

需要同时在 `blend_kit_server/` 跑起服务端，否则 `/api` 请求会失败。

## 检查

```bash
npm test                # vitest
npm run build           # tsc && vite build → dist/
npm run check:data      # master 数据自检
npm run check:consistency
```
