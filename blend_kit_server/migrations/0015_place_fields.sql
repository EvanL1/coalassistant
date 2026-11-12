-- 产地由一个自由文本字段 (region, 如 "山西灵石") 改成省 / 市 / 县三个字段, 取行政区划全称,
-- 估价对标时按 省 → 市 逐级比. region 列保留不删: 旧镜像回滚后照常读写, 对不上下面这张表的
-- 旧写法也不丢 (那样的行省市县留空, 核心仍按旧写法整串比较).
ALTER TABLE price_benchmarks ADD COLUMN IF NOT EXISTS province TEXT;
ALTER TABLE price_benchmarks ADD COLUMN IF NOT EXISTS city TEXT;
ALTER TABLE price_benchmarks ADD COLUMN IF NOT EXISTS county TEXT;
ALTER TABLE coal_quotes ADD COLUMN IF NOT EXISTS province TEXT;
ALTER TABLE coal_quotes ADD COLUMN IF NOT EXISTS city TEXT;
ALTER TABLE coal_quotes ADD COLUMN IF NOT EXISTS county TEXT;

-- 已有的旧写法: 只认下面列出的 (煤库与基准里出现过的写法, 加上山西各市). 已经填了省的行不动,
-- 所以重复执行无害. 一条语句、不建临时表: 不依赖迁移器是否把整份脚本包在一个事务里.
DO $$
DECLARE
    target TEXT;
BEGIN
    FOREACH target IN ARRAY ARRAY['price_benchmarks', 'coal_quotes'] LOOP
        EXECUTE format($sql$
            UPDATE %I AS place
            SET province = legacy.province, city = legacy.city, county = legacy.county
            FROM (VALUES
                ('山西', '山西省', NULL::TEXT, NULL::TEXT),
                ('陕西', '陕西省', NULL, NULL),
                ('内蒙古', '内蒙古自治区', NULL, NULL),
                ('山西太原', '山西省', '太原市', NULL),
                ('山西大同', '山西省', '大同市', NULL),
                ('山西阳泉', '山西省', '阳泉市', NULL),
                ('山西长治', '山西省', '长治市', NULL),
                ('山西晋城', '山西省', '晋城市', NULL),
                ('山西朔州', '山西省', '朔州市', NULL),
                ('山西晋中', '山西省', '晋中市', NULL),
                ('山西运城', '山西省', '运城市', NULL),
                ('山西忻州', '山西省', '忻州市', NULL),
                ('山西临汾', '山西省', '临汾市', NULL),
                ('山西吕梁', '山西省', '吕梁市', NULL),
                ('山西灵石', '山西省', '晋中市', '灵石县'),
                ('山西沁源', '山西省', '长治市', '沁源县'),
                ('山西古交', '山西省', '太原市', '古交市')
            ) AS legacy(region, province, city, county)
            WHERE place.region = legacy.region AND place.province IS NULL
        $sql$, target);
    END LOOP;
END $$;
