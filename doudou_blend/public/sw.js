// 豆哥配煤 Service Worker
// 策略:
//   - /api/*          → 直通网络, 不缓存 (后端数据)
//   - 其他 GET 请求   → 网络优先, 失败回 cache (离线兜底)
// 升级时清旧 cache, 立即接管.

const CACHE = "doudou-v2";

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((c) =>
        c.addAll(["/", "/index.html", "/icon.svg", "/manifest.webmanifest"]),
      )
      .catch(() => {}),
  );
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))),
    ),
  );
  self.clients.claim();
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  const url = new URL(req.url);

  if (url.pathname.startsWith("/api/")) return; // 后端调用直通

  if (req.method !== "GET") return;

  event.respondWith(
    fetch(req)
      .then((resp) => {
        if (resp && resp.status === 200 && resp.type === "basic") {
          const clone = resp.clone();
          caches.open(CACHE).then((c) => c.put(req, clone)).catch(() => {});
        }
        return resp;
      })
      .catch(async () => {
        const hit = await caches.match(req);
        if (hit) return hit;
        // SPA 路由: 找不到精确匹配回 index
        const root = await caches.match("/");
        if (root) return root;
        return new Response("offline", { status: 503 });
      }),
  );
});
