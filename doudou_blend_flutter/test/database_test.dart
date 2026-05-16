/// SQLite repository 集成测试: 用内存 DB 跑全套 CRUD.
library;

import 'package:flutter_test/flutter_test.dart';

import 'package:doudou_blend_flutter/data/coal_repository.dart';
import 'package:doudou_blend_flutter/data/customer_repository.dart';
import 'package:doudou_blend_flutter/data/database.dart';
import 'package:doudou_blend_flutter/data/settings_repository.dart';
import 'package:doudou_blend_flutter/models/business.dart';
import 'package:doudou_blend_flutter/models/coal.dart';

void main() {
  late AppDatabase db;

  setUp(() async {
    db = await AppDatabase.openInMemory();
  });

  tearDown(() async {
    await db.close();
  });

  group('schema', () {
    test('启动后 7 张表都建出来', () async {
      final rows = await db.db.rawQuery(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
      );
      final tables = rows.map((r) => r['name'] as String).toList();
      for (final t in [
        'contracts',
        'customers',
        'payments',
        'quotes',
        'shipments',
        'user_coals',
        'user_settings',
      ]) {
        expect(tables, contains(t), reason: '缺表: $t');
      }
    });
  });

  group('CustomerRepository', () {
    test('upsert → list → 命中', () async {
      final repo = CustomerRepository(db.db);
      const c = Customer(id: 'c1', name: '山东焦化', phone: '13800000000');
      await repo.upsert(c);
      final list = await repo.list();
      expect(list, hasLength(1));
      expect(list.first.id, 'c1');
      expect(list.first.name, '山东焦化');
      expect(list.first.phone, '13800000000');
    });

    test('同 id 二次 upsert 走更新, 不报错', () async {
      final repo = CustomerRepository(db.db);
      await repo.upsert(const Customer(id: 'c1', name: '原名'));
      await repo.upsert(const Customer(id: 'c1', name: '改名', contact: '王经理'));
      final list = await repo.list();
      expect(list, hasLength(1));
      expect(list.first.name, '改名');
      expect(list.first.contact, '王经理');
    });

    test('remove 真删', () async {
      final repo = CustomerRepository(db.db);
      await repo.upsert(const Customer(id: 'c1', name: 'x'));
      await repo.remove('c1');
      expect(await repo.list(), isEmpty);
    });

    test('多客户按 updated_at DESC 排序', () async {
      final repo = CustomerRepository(db.db);
      await repo.upsert(const Customer(id: 'c1', name: 'A'));
      await Future<void>.delayed(const Duration(milliseconds: 10));
      await repo.upsert(const Customer(id: 'c2', name: 'B'));
      final list = await repo.list();
      expect(list.first.id, 'c2');
      expect(list.last.id, 'c1');
    });
  });

  group('CoalRepository', () {
    test('add → list', () async {
      final repo = CoalRepository(db.db);
      await repo.add(const MasterCoalEntry(
        name: '测试煤',
        status: CoalStatus.draft,
        props: {'S': 0.5, 'A': 10.2},
      ));
      final list = await repo.list();
      expect(list, hasLength(1));
      expect(list.first.name, '测试煤');
      expect(list.first.props['S'], 0.5);
    });

    test('add 重名 → 抛 DuplicateCoalException (regression: 不能把别的错也归类成重名)',
        () async {
      final repo = CoalRepository(db.db);
      await repo.add(const MasterCoalEntry(
        name: '重名煤',
        status: CoalStatus.draft,
        props: {},
      ));
      expect(
        () => repo.add(const MasterCoalEntry(
          name: '重名煤',
          status: CoalStatus.draft,
          props: {},
        )),
        throwsA(isA<DuplicateCoalException>()),
      );
    });

    test('upsert 重名走更新', () async {
      final repo = CoalRepository(db.db);
      await repo.upsert(const MasterCoalEntry(
        name: '同名煤',
        status: CoalStatus.draft,
        props: {'S': 0.5},
      ));
      await repo.upsert(const MasterCoalEntry(
        name: '同名煤',
        status: CoalStatus.active,
        props: {'S': 0.7},
      ));
      final list = await repo.list();
      expect(list, hasLength(1));
      expect(list.first.status, CoalStatus.active);
      expect(list.first.props['S'], 0.7);
    });

    test('props 名字带空格不丢', () async {
      final repo = CoalRepository(db.db);
      await repo.add(MasterCoalEntry(
        name: '带空格煤',
        status: CoalStatus.draft,
        props: const {'S': 0.5},
        fob: 800,
      ));
      final list = await repo.list();
      expect(list.first.fob, 800);
    });

    test('remove', () async {
      final repo = CoalRepository(db.db);
      await repo.add(const MasterCoalEntry(
        name: '待删煤',
        status: CoalStatus.draft,
        props: {},
      ));
      await repo.remove('待删煤');
      expect(await repo.list(), isEmpty);
    });
  });

  group('SettingsRepository', () {
    test('get 不存在 → null', () async {
      final s = SettingsRepository(db.db);
      expect(await s.get('not-set'), isNull);
    });

    test('put → get → remove → get null', () async {
      final s = SettingsRepository(db.db);
      await s.put('k', '{"a":1}');
      expect(await s.get('k'), '{"a":1}');
      await s.remove('k');
      expect(await s.get('k'), isNull);
    });

    test('put 空字符串接受 (跟 worker 一致, typeof 不能拒空)', () async {
      final s = SettingsRepository(db.db);
      await s.put('empty', '');
      expect(await s.get('empty'), '');
    });

    test('put 同 key 走 upsert', () async {
      final s = SettingsRepository(db.db);
      await s.put('k', 'v1');
      await s.put('k', 'v2');
      expect(await s.get('k'), 'v2');
    });
  });
}
