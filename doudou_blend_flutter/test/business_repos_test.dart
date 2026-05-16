/// Quote / Contract / Payment / Shipment / History 仓库测试.
/// 含 regression: amount=0, net_tons=0, contract 级联删, history 100 截断.
library;

import 'package:flutter_test/flutter_test.dart';

import 'package:doudou_blend_flutter/data/contract_repository.dart';
import 'package:doudou_blend_flutter/data/database.dart';
import 'package:doudou_blend_flutter/data/history_repository.dart';
import 'package:doudou_blend_flutter/data/payment_repository.dart';
import 'package:doudou_blend_flutter/data/quote_repository.dart';
import 'package:doudou_blend_flutter/data/shipment_repository.dart';
import 'package:doudou_blend_flutter/models/business.dart';

void main() {
  late AppDatabase db;

  setUp(() async {
    db = await AppDatabase.openInMemory();
  });

  tearDown(() async {
    await db.close();
  });

  group('QuoteRepository', () {
    test('upsert + list', () async {
      final r = QuoteRepository(db.db);
      await r.upsert(const Quote(
        id: 'q1',
        customerId: 'c1',
        customerName: '客户A',
        recipe: {'煤A': 0.5, '煤B': 0.5},
        costCif: 1000,
        markup: 100,
        quotedPrice: 1100,
        status: QuoteStatus.draft,
      ));
      final list = await r.list();
      expect(list, hasLength(1));
      expect(list.first.quotedPrice, 1100);
      expect(list.first.recipe['煤A'], 0.5);
    });

    test('upsert 同 id 走更新, recipe 重新解析', () async {
      final r = QuoteRepository(db.db);
      await r.upsert(const Quote(
        id: 'q1',
        customerId: 'c1',
        customerName: '客户A',
        recipe: {'煤A': 1.0},
        costCif: 1000,
        markup: 100,
        quotedPrice: 1100,
        status: QuoteStatus.draft,
      ));
      await r.upsert(const Quote(
        id: 'q1',
        customerId: 'c1',
        customerName: '客户A',
        recipe: {'煤B': 0.6, '煤C': 0.4},
        costCif: 1200,
        markup: 50,
        quotedPrice: 1250,
        status: QuoteStatus.sent,
      ));
      final list = await r.list();
      expect(list, hasLength(1));
      expect(list.first.status, QuoteStatus.sent);
      expect(list.first.recipe, {'煤B': 0.6, '煤C': 0.4});
    });
  });

  group('ContractRepository - 级联删', () {
    test('删合同同时清 payments + shipments', () async {
      final contracts = ContractRepository(db.db);
      final payments = PaymentRepository(db.db);
      final shipments = ShipmentRepository(db.db);

      await contracts.upsert(const Contract(
        id: 'k1',
        customerId: 'c1',
        customerName: '客户',
        recipe: {'煤A': 1.0},
        unitPrice: 1000,
        totalTons: 100,
        totalAmount: 100000,
        firstPayPct: 80,
        firstPayAmount: 80000,
        tailPayAmount: 20000,
        status: ContractStatus.active,
      ));
      await payments.upsert(const Payment(
        id: 'p1',
        contractId: 'k1',
        kind: PaymentKind.first,
        amount: 80000,
        paidAt: '2026-05-16',
      ));
      await shipments.upsert(const Shipment(
        id: 's1',
        contractId: 'k1',
        netTons: 50,
        shippedAt: '2026-05-16',
        status: ShipmentStatus.shipped,
      ));

      await contracts.remove('k1');

      expect(await contracts.list(), isEmpty);
      expect(await payments.list(contractId: 'k1'), isEmpty);
      expect(await shipments.list(contractId: 'k1'), isEmpty);
    });
  });

  group('PaymentRepository', () {
    test('amount=0 接受 (regression)', () async {
      final r = PaymentRepository(db.db);
      await r.upsert(const Payment(
        id: 'p0',
        contractId: 'k1',
        kind: PaymentKind.other,
        amount: 0,
        paidAt: '2026-05-16',
      ));
      final list = await r.list();
      expect(list, hasLength(1));
      expect(list.first.amount, 0);
    });

    test('按 contract_id 过滤', () async {
      final r = PaymentRepository(db.db);
      await r.upsert(const Payment(
        id: 'p1',
        contractId: 'k1',
        kind: PaymentKind.first,
        amount: 100,
        paidAt: '2026-05-16',
      ));
      await r.upsert(const Payment(
        id: 'p2',
        contractId: 'k2',
        kind: PaymentKind.first,
        amount: 200,
        paidAt: '2026-05-16',
      ));
      final k1 = await r.list(contractId: 'k1');
      expect(k1, hasLength(1));
      expect(k1.first.contractId, 'k1');
    });
  });

  group('ShipmentRepository', () {
    test('net_tons=0 接受 (regression)', () async {
      final r = ShipmentRepository(db.db);
      await r.upsert(const Shipment(
        id: 's0',
        contractId: 'k1',
        netTons: 0,
        shippedAt: '2026-05-16',
        status: ShipmentStatus.shipped,
      ));
      final list = await r.list();
      expect(list, hasLength(1));
      expect(list.first.netTons, 0);
    });

    test('assay JSON encode/decode', () async {
      final r = ShipmentRepository(db.db);
      await r.upsert(const Shipment(
        id: 's1',
        contractId: 'k1',
        netTons: 50,
        shippedAt: '2026-05-16',
        status: ShipmentStatus.settled,
        assay: {'S': 0.5, 'A': 11.2, 'V': 28},
      ));
      final list = await r.list();
      expect(list.first.assay!['S'], 0.5);
      expect(list.first.assay!['A'], 11.2);
      expect(list.first.assay!['V'], 28);
    });
  });

  group('HistoryRepository', () {
    test('add 顺序: 最新在前', () async {
      final r = HistoryRepository(db.db);
      await r.add(const HistoryEntry(
        id: 'h1',
        title: 'old',
        recipeJson: '{}',
        costCifPerTon: 100,
        createdAt: '2026-05-15',
      ));
      await r.add(const HistoryEntry(
        id: 'h2',
        title: 'new',
        recipeJson: '{}',
        costCifPerTon: 200,
        createdAt: '2026-05-16',
      ));
      final list = await r.list();
      expect(list.first.id, 'h2');
      expect(list.last.id, 'h1');
    });

    test('cap 100 条 (LRU)', () async {
      final r = HistoryRepository(db.db);
      for (var i = 0; i < 120; i++) {
        await r.add(HistoryEntry(
          id: 'h$i',
          title: 'entry $i',
          recipeJson: '{}',
          costCifPerTon: i.toDouble(),
          createdAt: '2026-05-${(i % 28) + 1}',
        ));
      }
      final list = await r.list();
      expect(list.length, 100);
      expect(list.first.id, 'h119');
    });

    test('remove 按 id', () async {
      final r = HistoryRepository(db.db);
      await r.add(const HistoryEntry(
        id: 'h1',
        title: 'a',
        recipeJson: '{}',
        costCifPerTon: 100,
        createdAt: '2026-05-16',
      ));
      await r.add(const HistoryEntry(
        id: 'h2',
        title: 'b',
        recipeJson: '{}',
        costCifPerTon: 200,
        createdAt: '2026-05-16',
      ));
      await r.remove('h1');
      final list = await r.list();
      expect(list, hasLength(1));
      expect(list.first.id, 'h2');
    });
  });
}
