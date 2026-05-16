import 'package:sqflite/sqflite.dart';

import '../models/business.dart';

class CustomerRepository {
  CustomerRepository(this._db);

  final Database _db;

  Future<List<Customer>> list() async {
    final rows = await _db.query('customers',
        orderBy: 'updated_at DESC');
    return rows.map(Customer.fromRow).toList();
  }

  Future<void> upsert(Customer c) async {
    await _db.insert(
      'customers',
      c.toRow(),
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
    // sqflite 的 replace 会重置 created_at, 这里手动 patch 一下 updated_at
    await _db.update(
      'customers',
      {'updated_at': _now()},
      where: 'id = ?',
      whereArgs: [c.id],
    );
  }

  Future<void> remove(String id) async {
    await _db.delete('customers', where: 'id = ?', whereArgs: [id]);
  }

  static String _now() => DateTime.now().toUtc().toIso8601String();
}
