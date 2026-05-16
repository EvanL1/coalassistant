/// 跟 blend_kit_rs / doudou_blend types.ts 对齐的指标定义.
library;

/// 8 个煤质指标 key, 跟后端/算法约定一致.
enum IndicatorKey {
  s('S', '硫'),
  a('A', '灰'),
  v('V', '挥发'),
  g('G', '粘结'),
  y('Y', '胶质'),
  petro('petro', '岩相'),
  csr('CSR', '焦炭强度'),
  m('M', '水分');

  const IndicatorKey(this.code, this.labelZh);

  final String code;
  final String labelZh;

  static IndicatorKey? fromCode(String code) {
    for (final k in values) {
      if (k.code == code) return k;
    }
    return null;
  }
}

/// 跟 web 版 INDICATOR_ORDER 一致, 用于 UI 列出指标的顺序.
const List<IndicatorKey> indicatorOrder = IndicatorKey.values;

/// 兼容旧字符串 key (master json / db 里都是字符串) → 中文 label.
String indicatorLabelOf(String code) =>
    IndicatorKey.fromCode(code)?.labelZh ?? code;

/// Spec 的约束方向.
enum Direction {
  upper('Upper'),
  lower('Lower'),
  range('Range');

  const Direction(this.code);
  final String code;

  static Direction fromCode(String code) =>
      values.firstWhere((d) => d.code == code, orElse: () => Direction.range);
}
