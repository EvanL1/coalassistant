/// 数据模型层单测: JSON 来回 + enum 解析.
library;

import 'package:flutter_test/flutter_test.dart';

import 'package:doudou_blend_flutter/models/coal.dart';
import 'package:doudou_blend_flutter/models/indicators.dart';
import 'package:doudou_blend_flutter/models/spec.dart';

void main() {
  group('IndicatorKey', () {
    test('每个 key 都有中文 label', () {
      for (final k in IndicatorKey.values) {
        expect(k.labelZh, isNotEmpty);
      }
    });

    test('fromCode 命中 + miss 返回 null', () {
      expect(IndicatorKey.fromCode('S'), IndicatorKey.s);
      expect(IndicatorKey.fromCode('CSR'), IndicatorKey.csr);
      expect(IndicatorKey.fromCode('未知'), isNull);
    });

    test('indicatorLabelOf 兜底返回原字符串', () {
      expect(indicatorLabelOf('S'), '硫');
      expect(indicatorLabelOf('自定义指标'), '自定义指标');
    });
  });

  group('Direction', () {
    test('fromCode 三种方向 + 未知兜底 range', () {
      expect(Direction.fromCode('Upper'), Direction.upper);
      expect(Direction.fromCode('Lower'), Direction.lower);
      expect(Direction.fromCode('Range'), Direction.range);
      expect(Direction.fromCode('???'), Direction.range);
    });
  });

  group('CoalStatus', () {
    test('未知 status 兜底 draft', () {
      expect(CoalStatus.fromCode('verified'), CoalStatus.verified);
      expect(CoalStatus.fromCode('weird'), CoalStatus.draft);
    });
  });

  group('MasterCoalEntry', () {
    test('从 master JSON 解析', () {
      final j = {
        'name': '老山兰',
        'region': '山西',
        'coal_type': '肥煤',
        'status': 'verified',
        'props': {'S': 0.5, 'A': 10.2, 'V': 28.5},
        'fob': 800.0,
        'frt': 100.0,
        'confidence': {'S': 'high', 'A': 'medium'},
        'note': '主力煤种',
      };
      final c = MasterCoalEntry.fromJson(j);
      expect(c.name, '老山兰');
      expect(c.region, '山西');
      expect(c.status, CoalStatus.verified);
      expect(c.props['S'], 0.5);
      expect(c.props['A'], 10.2);
      expect(c.fob, 800);
      expect(c.confidence['S'], Confidence.high);
      expect(c.confidence['A'], Confidence.medium);
    });

    test('字段缺失时给安全默认', () {
      final c = MasterCoalEntry.fromJson({'name': '裸煤'});
      expect(c.name, '裸煤');
      expect(c.props, isEmpty);
      expect(c.fob, isNull);
      expect(c.status, CoalStatus.draft);
    });

    test('toJson 不含 null 字段, 但 status/name/props 必出', () {
      const c = MasterCoalEntry(
        name: '测试煤',
        status: CoalStatus.active,
        props: {'S': 0.5},
      );
      final j = c.toJson();
      expect(j['name'], '测试煤');
      expect(j['status'], 'active');
      expect(j['props'], {'S': 0.5});
      expect(j.containsKey('region'), false);
      expect(j.containsKey('fob'), false);
    });

    test('copyWith 只覆盖给定字段', () {
      const c = MasterCoalEntry(
        name: '原煤',
        status: CoalStatus.draft,
        props: {'S': 0.5},
        fob: 800,
      );
      final c2 = c.copyWith(fob: 900, status: CoalStatus.active);
      expect(c2.name, '原煤');
      expect(c2.fob, 900);
      expect(c2.status, CoalStatus.active);
      expect(c2.props['S'], 0.5);
    });

    test('props 里非数字值被过滤掉', () {
      final c = MasterCoalEntry.fromJson({
        'name': '坏数据',
        'props': {'S': 0.5, 'A': 'oops', 'V': null},
      });
      expect(c.props['S'], 0.5);
      expect(c.props.containsKey('A'), false);
      expect(c.props.containsKey('V'), false);
    });
  });

  group('Spec', () {
    test('JSON round-trip', () {
      const s = Spec(
        indicator: 'S',
        direction: Direction.upper,
        max: 0.8,
      );
      final j = s.toJson();
      final back = Spec.fromJson(j);
      expect(back.indicator, 'S');
      expect(back.direction, Direction.upper);
      expect(back.max, 0.8);
      expect(back.enabled, true);
    });

    test('copyWith', () {
      const s = Spec(indicator: 'A', direction: Direction.upper, max: 11);
      final s2 = s.copyWith(enabled: false, max: 12);
      expect(s2.enabled, false);
      expect(s2.max, 12);
      expect(s2.indicator, 'A');
    });
  });
}
