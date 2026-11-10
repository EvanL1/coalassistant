/**
 * 本机数据归属: 本地缓存记下是哪个账号的, 换账号登录时丢掉上一个账号的数据, 不上传也不导入.
 *
 * 服务端会话接口不返回用户名, 账号名取登录时输入的那个.
 */
import { PENALTY_TEMPLATE_KEY } from "./penaltyStorage";
import { KEY_HISTORY } from "./storage";

const PREFIX = "doudou_blend.";
const KEY_OWNER = "doudou_blend.owner.v1";

/**
 * 登出时保留的键. 采购扣款模板只存本机、云端没有副本; 本机历史还没导入云端
 * (登录时导入失败就留在本机) —— 登出清掉就永久丢了. 它们和账号名一起留着,
 * 换账号登录时 (claimLocalData) 才丢; 历史导入成功后由 storage.clearHistory 清.
 */
const KEPT_ON_LOGOUT = new Set([KEY_OWNER, PENALTY_TEMPLATE_KEY, KEY_HISTORY]);

function appKeys(store: Storage): string[] {
  const keys: string[] = [];
  for (let i = 0; i < store.length; i += 1) {
    const key = store.key(i);
    if (key?.startsWith(PREFIX)) keys.push(key);
  }
  return keys;
}

function removeAppKeys(store: Storage | undefined, keep: ReadonlySet<string>): void {
  if (!store) return;
  try {
    for (const key of appKeys(store)) {
      if (!keep.has(key)) store.removeItem(key);
    }
  } catch {
    // 存储不可用 (隐私模式等) 时没有可清的
  }
}

function sessionStore(): Storage | undefined {
  try {
    return typeof sessionStorage === "undefined" ? undefined : sessionStorage;
  } catch {
    return undefined;
  }
}

function localStore(): Storage | undefined {
  try {
    return typeof localStorage === "undefined" ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

/** 本机数据属于哪个账号; 本功能上线前留下的数据没有记录, 返回 null. */
export function getLocalOwner(): string | null {
  try {
    const owner = localStore()?.getItem(KEY_OWNER);
    return owner && owner.trim() ? owner : null;
  } catch {
    return null;
  }
}

/**
 * 登出: 清掉本应用的本地数据 (云端有副本的缓存、询价草稿).
 * 不派发变更事件 —— 同步监听此时还挂着, 派发会把空数据写回云端.
 */
export function clearLocalAppData(): void {
  removeAppKeys(localStore(), KEPT_ON_LOGOUT);
  removeAppKeys(sessionStore(), new Set());
}

/**
 * 登录成功后认领本机数据: 属于别的账号就整份丢掉 (含只存本机的扣款模板), 再记成当前账号.
 * 没有归属记录的旧数据归第一个登录的账号.
 */
export function claimLocalData(username: string): void {
  const name = username.trim();
  if (!name) return;
  const owner = getLocalOwner();
  if (owner != null && owner !== name) {
    removeAppKeys(localStore(), new Set());
    removeAppKeys(sessionStore(), new Set());
  }
  try {
    localStore()?.setItem(KEY_OWNER, name);
  } catch {
    // 存不进去时下次登录按无归属处理
  }
}
