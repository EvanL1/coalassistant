// 验证 WASM 模块加载后能正确求解 + 跟 Rust 原生 demo 输出一致.
// 期望: cost ≈ 1270.75 元/吨, 4 个 binding 约束.
import { readFileSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const wasmBytes = readFileSync(join(__dirname, 'pkg/blend_kit_wasm_bg.wasm'));
const { initSync, solveJson, getMasterJson, getVersion } = await import('./pkg/blend_kit_wasm.js');
initSync({ module: wasmBytes });

console.log('版本:', getVersion());

const master = JSON.parse(getMasterJson());
console.log('Master version:', master.version);
console.log('煤种总数:', master.coals.length);
console.log('verified (主力):', master.coals.filter(c => c.status === 'verified').length);

const verifiedCoals = master.coals
  .filter(c => c.status === 'verified')
  .map(c => ({ name: c.name, props: c.props, fob: c.fob, frt: c.frt }));

const request = {
  coals: verifiedCoals,
  specs: master.default_contract.specs,
  total_quantity: 3700,
  truncate_decimal: true,
};

const t0 = performance.now();
const resultJson = solveJson(JSON.stringify(request));
const elapsed = performance.now() - t0;
const result = JSON.parse(resultJson);

console.log('\n=== 求解结果 ===');
console.log(`耗时: ${elapsed.toFixed(2)}ms`);
console.log(`可行: ${result.ok}`);
if (result.ok) {
  console.log(`CIF 单价: ${result.cost.cif_per_ton.toFixed(2)} 元/吨`);
  console.log(`总金额:   ${result.cost.total_cif?.toFixed(2)} 元`);
  console.log('\n配方:');
  for (const o of result.orders) {
    console.log(`  ${o.coal.padEnd(8)} ${(o.ratio * 100).toFixed(2)}%  ${o.tons.toFixed(1)} 吨`);
  }
  console.log('\nbinding 约束:');
  for (const ic of result.indicator_check.filter(c => c.binding)) {
    console.log(`  ${ic.label_zh}: ${ic.value.toFixed(3)} (顶格)`);
  }
}
