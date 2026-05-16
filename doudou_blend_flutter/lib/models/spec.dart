import 'indicators.dart';

/// 合同指标约束 (Upper/Lower/Range).
class Spec {
  const Spec({
    required this.indicator,
    required this.direction,
    this.min,
    this.max,
    this.enabled = true,
  });

  final String indicator; // 跟 IndicatorKey.code 一致, 也允许自定义
  final Direction direction;
  final double? min;
  final double? max;
  final bool enabled;

  factory Spec.fromJson(Map<String, dynamic> j) => Spec(
        indicator: j['indicator'] as String,
        direction: Direction.fromCode((j['direction'] ?? 'Range') as String),
        min: (j['min'] as num?)?.toDouble(),
        max: (j['max'] as num?)?.toDouble(),
        enabled: (j['enabled'] as bool?) ?? true,
      );

  Map<String, dynamic> toJson() => {
        'indicator': indicator,
        'direction': direction.code,
        if (min != null) 'min': min,
        if (max != null) 'max': max,
        'enabled': enabled,
      };

  Spec copyWith({
    String? indicator,
    Direction? direction,
    double? min,
    double? max,
    bool? enabled,
  }) =>
      Spec(
        indicator: indicator ?? this.indicator,
        direction: direction ?? this.direction,
        min: min ?? this.min,
        max: max ?? this.max,
        enabled: enabled ?? this.enabled,
      );
}

/// 默认合同模板 (master JSON 里的 default_contract).
class DefaultContract {
  const DefaultContract({required this.name, required this.specs});

  final String name;
  final List<Spec> specs;

  factory DefaultContract.fromJson(Map<String, dynamic> j) => DefaultContract(
        name: j['name'] as String,
        specs: ((j['specs'] as List?) ?? [])
            .map((s) => Spec.fromJson(s as Map<String, dynamic>))
            .toList(),
      );
}
