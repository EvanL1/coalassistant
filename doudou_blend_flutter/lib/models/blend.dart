import 'coal.dart';
import 'spec.dart';

class BlendRequest {
  const BlendRequest({
    required this.coals,
    required this.specs,
    this.totalQuantity,
    this.truncateDecimal = false,
  });

  final List<Coal> coals;
  final List<Spec> specs;
  final double? totalQuantity;
  final bool truncateDecimal;

  Map<String, dynamic> toJson() => {
        'coals': coals.map((c) => c.toJson()).toList(),
        'specs': specs.map((s) => s.toJson()).toList(),
        if (totalQuantity != null) 'total_quantity': totalQuantity,
        'truncate_decimal': truncateDecimal,
      };
}

class CostBreakdown {
  const CostBreakdown({
    required this.fobPerTon,
    required this.frtPerTon,
    required this.cifPerTon,
    this.totalFob,
    this.totalFrt,
    this.totalCif,
  });

  final double fobPerTon;
  final double frtPerTon;
  final double cifPerTon;
  final double? totalFob;
  final double? totalFrt;
  final double? totalCif;

  factory CostBreakdown.fromJson(Map<String, dynamic> j) => CostBreakdown(
        fobPerTon: (j['fob_per_ton'] as num).toDouble(),
        frtPerTon: (j['frt_per_ton'] as num).toDouble(),
        cifPerTon: (j['cif_per_ton'] as num).toDouble(),
        totalFob: (j['total_fob'] as num?)?.toDouble(),
        totalFrt: (j['total_frt'] as num?)?.toDouble(),
        totalCif: (j['total_cif'] as num?)?.toDouble(),
      );
}

class OrderItem {
  const OrderItem({
    required this.coal,
    required this.ratio,
    this.tons,
    this.fobAmount,
    this.frtAmount,
    this.cifAmount,
  });

  final String coal;
  final double ratio;
  final double? tons;
  final double? fobAmount;
  final double? frtAmount;
  final double? cifAmount;

  factory OrderItem.fromJson(Map<String, dynamic> j) => OrderItem(
        coal: j['coal'] as String,
        ratio: (j['ratio'] as num).toDouble(),
        tons: (j['tons'] as num?)?.toDouble(),
        fobAmount: (j['fob_amount'] as num?)?.toDouble(),
        frtAmount: (j['frt_amount'] as num?)?.toDouble(),
        cifAmount: (j['cif_amount'] as num?)?.toDouble(),
      );
}

class IndicatorCheck {
  const IndicatorCheck({
    required this.indicator,
    required this.labelZh,
    required this.value,
    required this.binding,
    this.min,
    this.max,
    this.slack,
  });

  final String indicator;
  final String labelZh;
  final double value;
  final double? min;
  final double? max;
  final double? slack;
  final bool binding;

  factory IndicatorCheck.fromJson(Map<String, dynamic> j) => IndicatorCheck(
        indicator: j['indicator'] as String,
        labelZh: j['label_zh'] as String,
        value: (j['value'] as num).toDouble(),
        min: (j['min'] as num?)?.toDouble(),
        max: (j['max'] as num?)?.toDouble(),
        slack: (j['slack'] as num?)?.toDouble(),
        binding: j['binding'] as bool,
      );
}

class BlendResult {
  const BlendResult({
    required this.ok,
    required this.recipe,
    required this.orders,
    required this.indicatorCheck,
    required this.warnings,
    this.reason,
    this.cost,
  });

  final bool ok;
  final String? reason;
  final Map<String, double> recipe;
  final CostBreakdown? cost;
  final List<OrderItem> orders;
  final List<IndicatorCheck> indicatorCheck;
  final List<String> warnings;

  factory BlendResult.fromJson(Map<String, dynamic> j) => BlendResult(
        ok: j['ok'] as bool,
        reason: j['reason'] as String?,
        recipe: ((j['recipe'] as Map?) ?? {}).map(
          (k, v) => MapEntry(k.toString(), (v as num).toDouble()),
        ),
        cost: j['cost'] == null
            ? null
            : CostBreakdown.fromJson(j['cost'] as Map<String, dynamic>),
        orders: ((j['orders'] as List?) ?? [])
            .map((o) => OrderItem.fromJson(o as Map<String, dynamic>))
            .toList(),
        indicatorCheck: ((j['indicator_check'] as List?) ?? [])
            .map((c) => IndicatorCheck.fromJson(c as Map<String, dynamic>))
            .toList(),
        warnings: ((j['warnings'] as List?) ?? []).cast<String>(),
      );
}
