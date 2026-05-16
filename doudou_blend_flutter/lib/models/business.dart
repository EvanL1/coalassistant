/// Phase1 + Phase2 业务模型: 客户 / 报价 / 合同 / 收款 / 发货.
/// 跟 doudou_blend/src/types.ts 对齐, SQLite schema 字段一一映射.
library;

enum QuoteStatus {
  draft('draft'),
  sent('sent'),
  signed('signed'),
  lost('lost');

  const QuoteStatus(this.code);
  final String code;
  static QuoteStatus fromCode(String c) =>
      values.firstWhere((s) => s.code == c, orElse: () => QuoteStatus.draft);
}

enum ContractStatus {
  active('active'),
  completed('completed'),
  terminated('terminated');

  const ContractStatus(this.code);
  final String code;
  static ContractStatus fromCode(String c) => values
      .firstWhere((s) => s.code == c, orElse: () => ContractStatus.active);
}

enum PaymentKind {
  first('first'),
  tail('tail'),
  advance('advance'),
  other('other');

  const PaymentKind(this.code);
  final String code;
  static PaymentKind fromCode(String c) =>
      values.firstWhere((s) => s.code == c, orElse: () => PaymentKind.other);
}

enum ShipmentStatus {
  shipped('shipped'),
  arrived('arrived'),
  settled('settled');

  const ShipmentStatus(this.code);
  final String code;
  static ShipmentStatus fromCode(String c) => values
      .firstWhere((s) => s.code == c, orElse: () => ShipmentStatus.shipped);
}

class Customer {
  const Customer({
    required this.id,
    required this.name,
    this.contact,
    this.phone,
    this.note,
    this.createdAt,
    this.updatedAt,
  });

  final String id;
  final String name;
  final String? contact;
  final String? phone;
  final String? note;
  final String? createdAt;
  final String? updatedAt;

  factory Customer.fromRow(Map<String, Object?> r) => Customer(
        id: r['id'] as String,
        name: r['name'] as String,
        contact: r['contact'] as String?,
        phone: r['phone'] as String?,
        note: r['note'] as String?,
        createdAt: r['created_at'] as String?,
        updatedAt: r['updated_at'] as String?,
      );

  Map<String, Object?> toRow() => {
        'id': id,
        'name': name,
        'contact': contact,
        'phone': phone,
        'note': note,
      };

  Customer copyWith({
    String? name,
    String? contact,
    String? phone,
    String? note,
  }) =>
      Customer(
        id: id,
        name: name ?? this.name,
        contact: contact ?? this.contact,
        phone: phone ?? this.phone,
        note: note ?? this.note,
        createdAt: createdAt,
        updatedAt: updatedAt,
      );
}

class Quote {
  const Quote({
    required this.id,
    required this.customerId,
    required this.customerName,
    required this.recipe,
    required this.costCif,
    required this.markup,
    required this.quotedPrice,
    required this.status,
    this.totalTons,
    this.contractName,
    this.note,
    this.createdAt,
    this.updatedAt,
  });

  final String id;
  final String customerId;
  final String customerName;
  final Map<String, double> recipe;
  final double costCif;
  final double markup;
  final double quotedPrice;
  final double? totalTons;
  final String? contractName;
  final QuoteStatus status;
  final String? note;
  final String? createdAt;
  final String? updatedAt;
}

class Contract {
  const Contract({
    required this.id,
    required this.customerId,
    required this.customerName,
    required this.recipe,
    required this.unitPrice,
    required this.totalTons,
    required this.totalAmount,
    required this.firstPayPct,
    required this.firstPayAmount,
    required this.tailPayAmount,
    required this.status,
    this.quoteId,
    this.contractNo,
    this.billingLocation,
    this.prepayParty,
    this.signedAt,
    this.note,
    this.createdAt,
    this.updatedAt,
  });

  final String id;
  final String? quoteId;
  final String customerId;
  final String customerName;
  final String? contractNo;
  final String? billingLocation;
  final String? prepayParty;
  final Map<String, double> recipe;
  final double unitPrice;
  final double totalTons;
  final double totalAmount;
  final double firstPayPct;
  final double firstPayAmount;
  final double tailPayAmount;
  final String? signedAt;
  final ContractStatus status;
  final String? note;
  final String? createdAt;
  final String? updatedAt;
}

class Payment {
  const Payment({
    required this.id,
    required this.contractId,
    required this.kind,
    required this.amount,
    required this.paidAt,
    this.payer,
    this.method,
    this.voucherNo,
    this.note,
    this.createdAt,
  });

  final String id;
  final String contractId;
  final PaymentKind kind;
  final double amount;
  final String paidAt;
  final String? payer;
  final String? method;
  final String? voucherNo;
  final String? note;
  final String? createdAt;
}

class Shipment {
  const Shipment({
    required this.id,
    required this.contractId,
    required this.netTons,
    required this.shippedAt,
    required this.status,
    this.vehicleNo,
    this.grossTons,
    this.tareTons,
    this.arrivedAt,
    this.settledAt,
    this.settledAmount,
    this.assay,
    this.note,
    this.createdAt,
  });

  final String id;
  final String contractId;
  final String? vehicleNo;
  final double netTons;
  final double? grossTons;
  final double? tareTons;
  final String shippedAt;
  final String? arrivedAt;
  final String? settledAt;
  final double? settledAmount;
  final Map<String, double>? assay;
  final ShipmentStatus status;
  final String? note;
  final String? createdAt;
}

/// 保存历史方案 (今日屏点"保存"产出).
class HistoryEntry {
  const HistoryEntry({
    required this.id,
    required this.title,
    required this.recipeJson,
    required this.costCifPerTon,
    required this.createdAt,
    this.totalTons,
    this.note,
  });

  final String id;
  final String title;
  final String recipeJson; // {"煤名":比例,...}
  final double costCifPerTon;
  final double? totalTons;
  final String? note;
  final String createdAt;
}
