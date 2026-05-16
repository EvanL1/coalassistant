/// SQLite 初始化 + schema 自启 (跟 cloudflare/schema.sql 对齐).
///
/// 设计:
///  - 桌面/单测走 sqflite_common_ffi (内存或文件)
///  - 鸿蒙真机走 sqflite_ohos (通过 pubspec_overrides.yaml)
///  - 表结构跟 Cloudflare worker 完全一致, 方便以后 JSON 互导
library;

import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

class AppDatabase {
  AppDatabase._(this.db);

  final Database db;

  static AppDatabase? _instance;

  /// 全局单例. 主进程启动调一次, 测试里用 [openInMemory].
  static Future<AppDatabase> open() async {
    if (_instance != null) return _instance!;
    // 默认 sqflite (Android/iOS/鸿蒙 _ohos 端口), 桌面/CI 用 ffi.
    DatabaseFactory factory;
    if (_isDesktopOrTest) {
      sqfliteFfiInit();
      factory = databaseFactoryFfi;
    } else {
      factory = databaseFactory;
    }

    final dir = await getApplicationDocumentsDirectory();
    final path = p.join(dir.path, 'doudou_blend.db');
    final db = await factory.openDatabase(
      path,
      options: OpenDatabaseOptions(
        version: 1,
        onCreate: (db, _) async => _applySchema(db),
        onOpen: (db) async => _applySchema(db), // 幂等, 顺带兼容老 DB 升级
      ),
    );
    _instance = AppDatabase._(db);
    return _instance!;
  }

  /// 测试入口: 拿个内存 DB, 不写盘.
  static Future<AppDatabase> openInMemory() async {
    sqfliteFfiInit();
    final db = await databaseFactoryFfi.openDatabase(
      inMemoryDatabasePath,
      options: OpenDatabaseOptions(
        version: 1,
        onCreate: (db, _) async => _applySchema(db),
      ),
    );
    return AppDatabase._(db);
  }

  Future<void> close() => db.close();

  static bool get _isDesktopOrTest {
    // Flutter on desktop / tests: 用 ffi. 真机 (Android/iOS/鸿蒙) 用原生.
    try {
      return identical(0, 0.0) == false &&
          (const bool.fromEnvironment('dart.library.io') ||
              const bool.fromEnvironment('dart.library.ffi'));
    } catch (_) {
      return false;
    }
  }
}

Future<void> _applySchema(Database db) async {
  await db.execute('''
    CREATE TABLE IF NOT EXISTS user_coals (
      name TEXT PRIMARY KEY,
      region TEXT,
      coal_type TEXT,
      status TEXT NOT NULL DEFAULT 'draft',
      props_json TEXT NOT NULL DEFAULT '{}',
      fob REAL,
      frt REAL,
      note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
  ''');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_user_coals_status ON user_coals(status)');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_user_coals_updated ON user_coals(updated_at DESC)');

  await db.execute('''
    CREATE TABLE IF NOT EXISTS user_settings (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL,
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
  ''');

  await db.execute('''
    CREATE TABLE IF NOT EXISTS customers (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      contact TEXT, phone TEXT, note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
  ''');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_customers_updated ON customers(updated_at DESC)');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_customers_name ON customers(name)');

  await db.execute('''
    CREATE TABLE IF NOT EXISTS quotes (
      id TEXT PRIMARY KEY,
      customer_id TEXT NOT NULL,
      customer_name TEXT NOT NULL,
      recipe_json TEXT NOT NULL,
      cost_cif REAL NOT NULL,
      markup REAL NOT NULL DEFAULT 0,
      quoted_price REAL NOT NULL,
      total_tons REAL,
      contract_name TEXT,
      status TEXT NOT NULL DEFAULT 'draft',
      note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
  ''');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_quotes_customer ON quotes(customer_id)');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_quotes_status ON quotes(status)');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_quotes_updated ON quotes(updated_at DESC)');

  await db.execute('''
    CREATE TABLE IF NOT EXISTS contracts (
      id TEXT PRIMARY KEY,
      quote_id TEXT,
      customer_id TEXT NOT NULL,
      customer_name TEXT NOT NULL,
      contract_no TEXT,
      billing_location TEXT,
      prepay_party TEXT,
      recipe_json TEXT NOT NULL,
      unit_price REAL NOT NULL,
      total_tons REAL NOT NULL,
      total_amount REAL NOT NULL,
      first_pay_pct REAL NOT NULL DEFAULT 80,
      first_pay_amount REAL NOT NULL,
      tail_pay_amount REAL NOT NULL,
      signed_at TEXT,
      status TEXT NOT NULL DEFAULT 'active',
      note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
  ''');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_contracts_customer ON contracts(customer_id)');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_contracts_status ON contracts(status)');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_contracts_updated ON contracts(updated_at DESC)');

  await db.execute('''
    CREATE TABLE IF NOT EXISTS payments (
      id TEXT PRIMARY KEY,
      contract_id TEXT NOT NULL,
      kind TEXT NOT NULL DEFAULT 'first',
      amount REAL NOT NULL,
      paid_at TEXT NOT NULL,
      payer TEXT, method TEXT, voucher_no TEXT, note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
  ''');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_payments_contract ON payments(contract_id)');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_payments_paid_at ON payments(paid_at DESC)');

  await db.execute('''
    CREATE TABLE IF NOT EXISTS shipments (
      id TEXT PRIMARY KEY,
      contract_id TEXT NOT NULL,
      vehicle_no TEXT,
      net_tons REAL NOT NULL,
      gross_tons REAL, tare_tons REAL,
      shipped_at TEXT NOT NULL,
      arrived_at TEXT, settled_at TEXT,
      settled_amount REAL,
      assay_json TEXT,
      status TEXT NOT NULL DEFAULT 'shipped',
      note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
  ''');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_shipments_contract ON shipments(contract_id)');
  await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_shipments_shipped ON shipments(shipped_at DESC)');
}
