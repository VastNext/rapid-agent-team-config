// Visual & Integration verification script for rapid-agent-team-config frontend logic and IPC
const fs = require('fs');
const path = require('path');

console.log("=== 1. Checking JS & HTML files exist ===");
const htmlPath = path.join(__dirname, '../src/frontend/index.html');
const jsPath = path.join(__dirname, '../src/frontend/app.js');
const cssPath = path.join(__dirname, '../src/frontend/style.css');

if (!fs.existsSync(htmlPath) || !fs.existsSync(jsPath) || !fs.existsSync(cssPath)) {
  console.error("Missing frontend files!");
  process.exit(1);
}

const html = fs.readFileSync(htmlPath, 'utf8');
const js = fs.readFileSync(jsPath, 'utf8');
const mainPath = path.join(__dirname, '../src/main.rs');
const mainRs = fs.readFileSync(mainPath, 'utf8');

console.log("=== 2. Checking XSS & HTML escaping ===");
if (!js.includes('escapeHtml(')) {
  console.error("Missing escapeHtml function in app.js!");
  process.exit(1);
}

if (!js.includes('copyToClipboard(')) {
  console.error("Missing copyToClipboard helper!");
  process.exit(1);
}

console.log("=== 3. Checking previewPlan real changes only & confirm sanitized items ===");
if (!js.includes('realChangeItems') || !js.includes('orig !== newModel')) {
  console.error("previewPlan does not strictly filter real changes!");
  process.exit(1);
}

if (!js.includes('sanitizedItems') || !js.includes('it.new_model && it.new_model.trim().length > 0')) {
  console.error("confirmApply does not sanitize empty items!");
  process.exit(1);
}

console.log("=== 4. Checking UI buttons, Diff copy & SHA-256 copy ===");
if (!html.includes('btn-copy-sha256-link')) {
  console.error("Missing btn-copy-sha256-link in index.html!");
  process.exit(1);
}

if (!html.includes('btn-copy-diff') || !js.includes('copyCurrentDiff')) {
  console.error("Missing btn-copy-diff in index.html or copyCurrentDiff in app.js!");
  process.exit(1);
}

console.log("=== 5. Checking App Config isolation & Fallback UI guidance ===");
if (!js.includes('save_app_config') || !js.includes('get_app_config')) {
  console.error("Missing isolated app config save/load in frontend!");
  process.exit(1);
}

if (!js.includes('VastNext/opencode-rapid-agent-team')) {
  console.error("Missing official VastNext/opencode-rapid-agent-team repository in fetchRelease!");
  process.exit(1);
}

console.log("=== 6. Checking preview modal, copy buttons and manual ZIP flow ===");
// 完整预览：Diff 弹窗 + 复制按钮
if (!html.includes('diff-modal') || !js.includes('renderDiffModal') || !js.includes('copyCurrentDiff')) {
  console.error("Missing full diff preview modal or copy button!");
  process.exit(1);
}
// 手动 zip 流程
if (!html.includes('btn-install-zip') || !js.includes('installFromZip') || !js.includes('generate_zip_install_plan')) {
  console.error("Missing manual ZIP install flow!");
  process.exit(1);
}

console.log("=== 7. Checking dynamic innerHTML is escaped (no raw injection) ===");
// card.innerHTML / item.innerHTML / infoBox.innerHTML 中的动态值必须经过 escapeHtml
const innerHtmlBlocks = js.match(/\.innerHTML\s*=\s*`[^`]*`/g) || [];
for (const block of innerHtmlBlocks) {
  // 检查每个含 ${...} 的块是否都使用 escapeHtml 包裹（允许少数纯常量/数字/内部自造串）
  const interpolations = block.match(/\$\{([^}]+)\}/g) || [];
  for (const interp of interpolations) {
    const expr = interp.slice(2, -1);
    const isSafe = /escapeHtml\(/.test(expr) ||
      /^(has_platform_asset|isSel|selectOptions|'selected'|''|selectedModel|models\.length)$/.test(expr.trim()) ||
      /^selectOptions\s*\|\|/.test(expr.trim()) ||
      /^\d+$/.test(expr.trim()) ||
      // 三元表达式：仅当两个分支都是字面量字符串/空串时才安全
      (/^\s*[^?]+\?\s*('[^']*'|"")\s*:\s*('[^']*'|"")\s*$/.test(expr));
    if (!isSafe) {
      console.error(`Unsafe interpolation in innerHTML: ${interp}`);
      process.exit(1);
    }
  }
}

console.log("=== 8. Checking buttons referenced in JS exist in HTML ===");
const jsButtonIds = [...js.matchAll(/getElementById\('(btn-[^']+)'\)/g)].map(m => m[1]);
for (const id of jsButtonIds) {
  if (!html.includes(`id="${id}"`)) {
    console.error(`JS references button '${id}' that does not exist in index.html!`);
    process.exit(1);
  }
}

console.log("=== 9. Checking model search / select filtering & dir-install confirmation parity ===");
// 可搜索选择：输入框 + 分隔符容错 + 字符顺序模糊匹配
if (!js.includes('model-filter-input') || !js.includes('filterModelOptions') || !js.includes('fuzzyMatch') || !js.includes('normalizeSearchText') || !js.includes('split(/\\s+/)')) {
  console.error("Missing searchable model select (model-filter-input / filterModelOptions)!");
  process.exit(1);
}
if (!js.includes('requestAnimationFrame') || !js.includes('scheduleModelFilter')) {
  console.error("Global model filtering must be coalesced to animation frames!");
  process.exit(1);
}
// installFromDir 需与 ZIP 流程一致的安装前确认（安装计划 + 用户确认）
if (!js.includes('installFromDir') || !js.includes('confirm(')) {
  console.error("installFromDir must show user confirmation before installing!");
  process.exit(1);
}

console.log("=== 10. Checking Release page copy & open per-address with failure feedback ===");
// 各地址旁提供复制 + 打开：Release 页面、下载直链、SHA-256 地址
if (!html.includes('btn-copy-release-link') || !html.includes('btn-copy-sha256-link')) {
  console.error("Missing copy buttons next to release / sha256 addresses!");
  process.exit(1);
}
if (!html.includes('btn-open-release-page')) {
  console.error("Missing open-release-page button!");
  process.exit(1);
}
// 复制失败需反馈（copyToClipboard catch 分支显示错误 toast）
if (!js.includes('复制失败') || !js.includes('catch (err)')) {
  console.error("copyToClipboard must surface copy failures!");
  process.exit(1);
}

console.log("=== 11. Checking official default repository & install wizard for missing team ===");
if (!js.includes('VastNext/opencode-rapid-agent-team')) {
  console.error("Missing official default VastNext/opencode-rapid-agent-team repo!");
  process.exit(1);
}

console.log("=== 12. Checking consent gate and explicit config paths ===");
if (!html.includes('consent-screen') || !html.includes('config-path-input') ||
    !html.includes('btn-confirm-scan') || !js.includes('scanConfirmed') ||
    !js.includes('config_paths') || !js.includes('scan_environment')) {
  console.error("Missing pre-scan consent gate or explicit config path flow!");
  process.exit(1);
}

console.log("=== 13. Checking Windows GUI subsystem and hidden WebView startup ===");
if (!mainRs.includes('windows_subsystem') || !mainRs.includes('.with_visible(false)') ||
    !mainRs.includes('PageLoadEvent::Finished')) {
  console.error("Missing Windows GUI subsystem or hidden-until-loaded startup guard!");
  process.exit(1);
}
const initialLoadBlock = js.match(/DOMContentLoaded[\s\S]*?\n\}\);/)?.[0] || '';
if (initialLoadBlock.includes('loadSavedAppConfig(')) {
  console.error("App configuration must not load before consent confirmation!");
  process.exit(1);
}
// 缺少 Rapid Team 时安装向导应出现并引导（banner 文案 / 安装按钮）
if (!js.includes('btn-install-wizard') || !js.includes('一键安装 Rapid Team')) {
  console.error("Install wizard entry missing when Rapid Team absent!");
  process.exit(1);
}

console.log("✅ All Frontend & JS static validation tests PASSED!");
