import 'dart:convert';

import 'package:sqflite/sqflite.dart';

import '../models/business.dart';

class ShipmentRepository {
  ShipmentRepository(this._db);

  final Database _db;

  Future<List<Shipment>> list({String? contractId}) async {
    final rows = await _db.query(
      'shipments',
      where: contractId == null ? null : 'contract_id = ?',
      whereArgs: contractId == null ? null : [contractId],
      orderBy: 'shipped_at DESC',
    );
    return rows.map(_fromRow).toList();
  }

  /// 注意: net_tons = 0 是合法值 (例如登记空车占位); 用 null 判空, 不用
  /// falsy. 跟 worker 修过的 amount bug 同一类陷阱.
  Future<void> upsert(Shipment s) async {
    await _db.insert(
      'shipments',
      _toRow(s),
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
  }

  Future<void> remove(String id) async {
    await _db.delete('shipments', where: 'id = ?', whereArgs: [id]);
  }

  static Shipment _fromRow(Map<String, Object?> r) {
    return Shipment(
      id: r['id'] as String,
      contractId: r['contract_id'] as String,
      vehicleNo: r['vehicle_no'] as String?,
      netTons: (r['net_tons'] as num).toDouble(),
      grossTons: (r['gross_tons'] as num?)?.toDouble(),
      tareTons: (r['tare_tons'] as num?)?.toDouble(),
      shippedAt: r['shipped_at'] as String,
      arrivedAt: r['arrived_at'] as String?,
      settledAt: r['settled_at'] as String?,
      settledAmount: (r['settled_amount'] as num?)?.toDouble(),
      assay: _decodeAssay(r['assay_json'] as String?),
      status: ShipmentStatus.fromCode((r['status'] ?? 'shipped') as String),
      note: r['note'] as String?,
      createdAt: r['created_at'] as String?,
    );
  }

  static Map<String, Object?> _toRow(Shipment s) => {
        'id': s.id,
        'contract_id': s.contractId,
        'vehicle_no': s.vehicleNo,
        'net_tons': s.netTons,
        'gross_tons': s.grossTons,
        'tare_tons': s.tareTons,
        'shipped_at': s.shippedAt,
        'arrived_at': s.arrivedAt,
        'settled_at': s.settledAt,
        'settled_amount': s.settledAmount,
        'assay_json': s.assay == null ? null : jsonEncode(s.assay),
        'status': s.status.code,
        'note': s.note,
      };

  static Map<String, double>? _decodeAssay(String? raw) {
    if (raw == null || raw.isEmpty) return null;
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map) {
        return {
          for (final e in decoded.entries)
            e.key.toString(): (e.value as num).toDouble(),
        };
      }
    } catch (_) {
      // fall through
    }
    return null;
  }
}
