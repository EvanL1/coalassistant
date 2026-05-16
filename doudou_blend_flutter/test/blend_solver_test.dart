/// BlendSolver Dart fallback 测试 (FFI 路径需要真机 + libblend_kit_rs.so,
/// CI 跑不了).
library;

import 'package:flutter_test/flutter_test.dart';

import 'package:doudou_blend_flutter/models/blend.dart';
import 'package:doudou_blend_flutter/models/coal.dart';
import 'package:doudou_blend_flutter/services/blend_solver.dart';

void main() {
  final solver = DartFallbackBlendSolver();

  test('空 coals → ok=false', () async {
    final r = await solver.solve(const BlendRequest(coals: [], specs: []));
    expect(r.ok, false);
    expect(r.reason, contains('no coals'));
  });

  test('均分 + 成本算对', () async {
    final r = await solver.solve(const BlendRequest(
      coals: [
        Coal(name: '煤A', props: {'S': 0.5}, fob: 800, frt: 100),
        Coal(name: '煤B', props: {'S': 0.6}, fob: 1000, frt: 100),
      ],
      specs: [],
    ));
    expect(r.ok, true);
    expect(r.recipe['煤A'], closeTo(0.5, 1e-9));
    expect(r.recipe['煤B'], closeTo(0.5, 1e-9));
    expect(r.cost!.fobPerTon, closeTo(900, 1e-9));
    expect(r.cost!.frtPerTon, closeTo(100, 1e-9));
    expect(r.cost!.cifPerTon, closeTo(1000, 1e-9));
  });

  test('给 total_quantity 时算总价', () async {
    final r = await solver.solve(const BlendRequest(
      coals: [
        Coal(name: '煤A', props: {}, fob: 800, frt: 100),
      ],
      specs: [],
      totalQuantity: 1000,
    ));
    expect(r.cost!.totalCif, closeTo(900000, 1e-9));
    expect(r.orders.first.tons, 1000);
  });

  test('警告里明确提示是 fallback', () async {
    final r = await solver.solve(const BlendRequest(
      coals: [
        Coal(name: '煤A', props: {}, fob: 800, frt: 100),
      ],
      specs: [],
    ));
    expect(r.warnings, isNotEmpty);
    expect(r.warnings.first, contains('fallback'));
  });
}
