//! 数据更新接口的 API 密钥: 生成、哈希、展示前缀.
//!
//! 密钥在管理端页面生成, 明文只在创建响应里出现一次, 库里只留 SHA-256。
//! 因此忘了就只能销毁重建 —— 这是有意的, 不是缺陷。

use rand::RngCore;
use sha2::{Digest, Sha256};

/// 明文密钥前缀. 让人一眼认出这是什么东西, 也方便日后在日志/代码里搜。
const KEY_PREFIX: &str = "dk_";
/// 随机部分的字节数. 256 位, 远超暴力枚举可行范围。
const KEY_RANDOM_BYTES: usize = 32;
/// 管理端列表里展示的前缀长度 (含 `dk_`), 用于认领某一把, 不足以还原密钥。
const DISPLAY_PREFIX_LEN: usize = 11;

/// 新生成的明文密钥. 只在创建时存在一次。
pub struct GeneratedKey {
    pub plaintext: String,
    pub hash: String,
    pub prefix: String,
}

/// 生成一把新密钥. 用 OS 的 CSPRNG, 不要换成 thread_rng 之外的弱随机源。
pub fn generate() -> GeneratedKey {
    let mut bytes = [0u8; KEY_RANDOM_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let plaintext = format!("{KEY_PREFIX}{}", hex(&bytes));
    let hash = hash_key(&plaintext);
    let prefix = display_prefix(&plaintext);
    GeneratedKey {
        plaintext,
        hash,
        prefix,
    }
}

/// 对明文密钥做 SHA-256, 返回十六进制.
///
/// 这里用 SHA-256 而不是 argon2/bcrypt 是合适的: 那些慢哈希是为**低熵**的人造密码
/// 设计的, 用来拖慢字典攻击; 而这把钥匙是 256 位随机值, 不存在字典可猜, 慢哈希只会
/// 让每次鉴权都变慢。
pub fn hash_key(plaintext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hex(&hasher.finalize())
}

/// 管理端列表里展示用的前缀, 例如 `dk_3f9a2b1`.
pub fn display_prefix(plaintext: &str) -> String {
    plaintext.chars().take(DISPLAY_PREFIX_LEN).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_has_prefix_and_full_entropy() {
        let key = generate();
        assert!(key.plaintext.starts_with(KEY_PREFIX));
        // dk_ + 32 字节的十六进制
        assert_eq!(key.plaintext.len(), KEY_PREFIX.len() + KEY_RANDOM_BYTES * 2);
        assert!(key.plaintext[KEY_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    /// 两次生成绝不能撞 —— 撞了说明随机源坏了.
    #[test]
    fn generated_keys_are_unique() {
        let keys: std::collections::HashSet<String> =
            (0..64).map(|_| generate().plaintext).collect();
        assert_eq!(keys.len(), 64);
    }

    #[test]
    fn hash_is_stable_and_differs_per_key() {
        let a = generate();
        let b = generate();
        assert_eq!(a.hash, hash_key(&a.plaintext), "同一明文必须哈希一致");
        assert_ne!(a.hash, b.hash);
        assert_eq!(a.hash.len(), 64, "SHA-256 十六进制应为 64 字符");
    }

    /// 哈希不得包含明文 —— 库泄露时不能反推出密钥.
    #[test]
    fn hash_does_not_leak_plaintext() {
        let key = generate();
        let random_part = &key.plaintext[KEY_PREFIX.len()..];
        assert!(!key.hash.contains(random_part));
    }

    /// 展示前缀足够短, 不足以还原密钥.
    #[test]
    fn display_prefix_is_short() {
        let key = generate();
        assert_eq!(key.prefix.len(), DISPLAY_PREFIX_LEN);
        assert!(key.plaintext.starts_with(&key.prefix));
        assert!(
            key.prefix.len() < key.plaintext.len() / 4,
            "前缀不应泄露密钥的可观察部分"
        );
    }
}
