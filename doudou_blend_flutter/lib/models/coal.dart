/// 煤种数据模型. 跟 doudou_blend/src/types.ts 的 MasterCoalEntry / Coal 对齐.
library;

enum CoalStatus {
  verified('verified'),
  active('active'),
  draft('draft'),
  incomplete('incomplete'),
  archived('archived');

  const CoalStatus(this.code);
  final String code;

  static CoalStatus fromCode(String code) =>
      values.firstWhere((s) => s.code == code, orElse: () => CoalStatus.draft);
}

enum Confidence { high, medium, low }

/// Master 煤库 + 用户新增 都用这个 entry.
/// props 用 String key (跟 indicators.dart 的 code 一致).
class MasterCoalEntry {
  const MasterCoalEntry({
    required this.name,
    required this.status,
    required this.props,
    this.region,
    this.coalType,
    this.fob,
    this.frt,
    this.confidence = const {},
    this.note,
  });

  final String name;
  final String? region;
  final String? coalType;
  final CoalStatus status;
  final Map<String, double> props;
  final double? fob;
  final double? frt;
  final Map<String, Confidence> confidence;
  final String? note;

  factory MasterCoalEntry.fromJson(Map<String, dynamic> j) => MasterCoalEntry(
        name: j['name'] as String,
        region: j['region'] as String?,
        coalType: j['coal_type'] as String?,
        status: CoalStatus.fromCode((j['status'] ?? 'draft') as String),
        props: _numMap(j['props']),
        fob: _toDouble(j['fob']),
        frt: _toDouble(j['frt']),
        confidence: _confidenceMap(j['confidence']),
        note: j['note'] as String?,
      );

  Map<String, dynamic> toJson() => {
        'name': name,
        if (region != null) 'region': region,
        if (coalType != null) 'coal_type': coalType,
        'status': status.code,
        'props': props,
        if (fob != null) 'fob': fob,
        if (frt != null) 'frt': frt,
        if (note != null) 'note': note,
      };

  MasterCoalEntry copyWith({
    String? name,
    String? region,
    String? coalType,
    CoalStatus? status,
    Map<String, double>? props,
    double? fob,
    double? frt,
    String? note,
  }) =>
      MasterCoalEntry(
        name: name ?? this.name,
        region: region ?? this.region,
        coalType: coalType ?? this.coalType,
        status: status ?? this.status,
        props: props ?? this.props,
        fob: fob ?? this.fob,
        frt: frt ?? this.frt,
        confidence: confidence,
        note: note ?? this.note,
      );
}

/// 求解器输入用的精简煤种结构.
class Coal {
  const Coal({
    required this.name,
    required this.props,
    required this.fob,
    required this.frt,
  });

  final String name;
  final Map<String, double> props;
  final double fob;
  final double frt;

  Map<String, dynamic> toJson() => {
        'name': name,
        'props': props,
        'fob': fob,
        'frt': frt,
      };
}

// ============================================================
// 工具
// ============================================================

Map<String, double> _numMap(dynamic raw) {
  if (raw is! Map) return const {};
  final out = <String, double>{};
  raw.forEach((k, v) {
    final d = _toDouble(v);
    if (d != null) out[k.toString()] = d;
  });
  return out;
}

Map<String, Confidence> _confidenceMap(dynamic raw) {
  if (raw is! Map) return const {};
  final out = <String, Confidence>{};
  raw.forEach((k, v) {
    out[k.toString()] = switch (v) {
      'high' => Confidence.high,
      'medium' => Confidence.medium,
      _ => Confidence.low,
    };
  });
  return out;
}

double? _toDouble(dynamic v) {
  if (v == null) return null;
  if (v is num) return v.toDouble();
  if (v is String) return double.tryParse(v);
  return null;
}
