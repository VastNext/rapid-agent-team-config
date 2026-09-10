// Rapid Agent Team Configurator Frontend Logic

let appState = {
  scanResult: null,
  selectedScope: 'global',
  projectPath: null,
  selectedModels: {}, // agent_name -> new_model
  proxyConfig: {
    enabled: true,
    proxy_url: 'http://127.0.0.1:7890'
  },
  currentPlan: null,
  latestRelease: null,
  configPaths: [],
  scanConfirmed: false,
  modelIndex: [],
  activeModelInput: null,
  activeModelAgent: null,
  modelPickerTimer: null,
  searchFrame: null
};

// Safe HTML escaping helper
function escapeHtml(str) {
  if (str === null || str === undefined) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}

// Copy helper with robust error handling
async function copyToClipboard(text, successMsg = '已复制到剪贴板') {
  if (!text) return;
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
    } else {
      const textArea = document.createElement('textarea');
      textArea.value = text;
      textArea.style.position = 'fixed';
      textArea.style.left = '-999999px';
      textArea.style.top = '-999999px';
      document.body.appendChild(textArea);
      textArea.focus();
      textArea.select();
      const successful = document.execCommand('copy');
      textArea.remove();
      if (!successful) throw new Error('execCommand copy failed');
    }
    showToast(successMsg, 'success');
  } catch (err) {
    showToast(`复制失败: ${err.message}`, 'error');
  }
}

// Safe RPC invoker to Rust backend
async function callRust(action, payload = {}) {
  try {
    const resp = await fetch('/api/ipc', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({ action, payload })
    });
    if (!resp.ok) {
      throw new Error(`HTTP ${resp.status}: ${resp.statusText}`);
    }
    const json = await resp.json();
    if (json.error) {
      throw new Error(json.error);
    }
    return json.data;
  } catch (fetchErr) {
    // Fallback to window.ipc if running under legacy IPC shim
    if (window.ipc) {
      return new Promise((resolve, reject) => {
        const callbackId = 'cb_' + Date.now() + '_' + Math.floor(Math.random() * 10000);
        window[callbackId] = function(res) {
          delete window[callbackId];
          if (res.error) {
            reject(new Error(res.error));
          } else {
            resolve(res.data);
          }
        };
        window.ipc.postMessage(JSON.stringify({ action, callbackId, payload }));
      });
    }
    console.warn("IPC not available, running in mock/browser mode", action, payload);
    return { mock: true };
  }
}

function showToast(message, type = 'info') {
  const container = document.getElementById('toast-container');
  if (!container) return;
  const toast = document.createElement('div');
  toast.className = `toast ${type}`;
  toast.textContent = message;
  container.appendChild(toast);
  setTimeout(() => {
    toast.remove();
  }, 4000);
}

// 模型搜索使用宽松模糊匹配：忽略常见分隔符，并允许关键词字符按顺序出现。
function normalizeSearchText(value) {
  return String(value || '').toLowerCase().replace(/[\s\/_().:-]+/g, '');
}

function parseSearchTokens(query) {
  return String(query || '')
    .toLowerCase()
    .split(/\s+/)
    .map(normalizeSearchText)
    .filter(Boolean);
}

function fuzzyMatch(value, query) {
  const target = normalizeSearchText(value);
  const tokens = parseSearchTokens(query);
  return tokens.every(token => {
    if (target.includes(token)) return true;
    let cursor = 0;
    for (const character of token) {
      cursor = target.indexOf(character, cursor);
      if (cursor === -1) return false;
      cursor += 1;
    }
    return true;
  });
}

// Initial Load
window.addEventListener('DOMContentLoaded', async () => {
  setupEventHandlers();
  // 前置确认通过前不读取任何本地配置，包括应用自己的代理配置。
  renderConfigPathList();
});

async function loadSavedAppConfig() {
  try {
    const cfg = await callRust('get_app_config');
    if (cfg && cfg.proxy) {
      appState.proxyConfig = cfg.proxy;
      const proxyEnabledEl = document.getElementById('proxy-enabled');
      if (proxyEnabledEl) proxyEnabledEl.checked = cfg.proxy.enabled;
      const proxyUrlEl = document.getElementById('proxy-url');
      if (proxyUrlEl) proxyUrlEl.value = cfg.proxy.proxy_url || 'http://127.0.0.1:7890';
    }
  } catch (err) {
    console.warn('Failed to load app config:', err);
  }
}

function setupEventHandlers() {
  // Tabs
  document.querySelectorAll('.tab-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
      document.querySelectorAll('.tab-pane').forEach(p => p.classList.remove('active'));
      btn.classList.add('active');
      const tabId = btn.getAttribute('data-tab');
      const targetPane = document.getElementById(tabId);
      if (targetPane) targetPane.classList.add('active');
    });
  });

  // Scope Toggle
  const scopeGlobalEl = document.getElementById('scope-global');
  if (scopeGlobalEl) {
    scopeGlobalEl.addEventListener('change', () => {
      appState.selectedScope = 'global';
      const btnSelProj = document.getElementById('btn-select-project');
      if (btnSelProj) btnSelProj.style.display = 'none';
      refreshState();
    });
  }

  const scopeProjEl = document.getElementById('scope-project');
  if (scopeProjEl) {
    scopeProjEl.addEventListener('change', () => {
      appState.selectedScope = 'project';
      const btnSelProj = document.getElementById('btn-select-project');
      if (btnSelProj) btnSelProj.style.display = 'inline-flex';
      if (!appState.projectPath) {
        chooseProjectFolder();
      } else {
        refreshState();
      }
    });
  }

  // Safe Button bindings
  const safeBind = (id, event, handler) => {
    const el = document.getElementById(id);
    if (el) el.addEventListener(event, handler);
  };

  safeBind('btn-refresh', 'click', refreshState);
  safeBind('btn-confirm-scan', 'click', confirmScan);
  safeBind('btn-select-config-path', 'click', chooseConfigPath);
  safeBind('btn-add-config-path', 'click', addConfigPath);
  safeBind('btn-select-project', 'click', chooseProjectFolder);
  safeBind('btn-install-wizard', 'click', () => {
    const tabInstallBtn = document.querySelector('[data-tab="tab-install"]');
    if (tabInstallBtn) tabInstallBtn.click();
  });

  const filterInput = document.getElementById('model-filter-input');
  if (filterInput) {
    filterInput.addEventListener('input', (e) => {
      scheduleModelFilter(e.target.value);
    });
  }

  safeBind('btn-batch-apply-default', 'click', autoMatchModels);
  safeBind('btn-preview-plan', 'click', previewPlan);

  // Modal
  safeBind('btn-close-modal', 'click', closeModal);
  safeBind('btn-cancel-apply', 'click', closeModal);
  safeBind('btn-confirm-apply', 'click', confirmApply);
  safeBind('btn-copy-diff', 'click', copyCurrentDiff);
  const sharedModelOptions = document.getElementById('shared-model-options');
  if (sharedModelOptions) {
    sharedModelOptions.addEventListener('mousedown', event => {
      if (event.target.closest('.model-option')) event.preventDefault();
    });
    sharedModelOptions.addEventListener('click', event => {
      const option = event.target.closest('.model-option');
      if (!option || !appState.activeModelInput) return;
      appState.activeModelInput.value = option.dataset.model;
      appState.selectedModels[appState.activeModelAgent] = option.dataset.model;
      sharedModelOptions.classList.remove('open');
    });
  }

  // Install handlers
  safeBind('btn-install-zip', 'click', installFromZip);
  safeBind('btn-install-dir', 'click', installFromDir);
  safeBind('btn-fetch-release', 'click', fetchRelease);
  safeBind('btn-download-install-release', 'click', downloadAndInstallRelease);
  safeBind('btn-copy-release-link', 'click', copyReleaseLink);
  safeBind('btn-copy-sha256-link', 'click', copySha256Link);

  // Proxy
  safeBind('btn-test-proxy', 'click', testProxy);
  safeBind('btn-save-proxy', 'click', saveProxyConfig);
}

async function refreshState() {
  if (!appState.scanConfirmed) return;
  try {
    const res = await callRust('scan_environment', {
      project_path: appState.projectPath,
      use_project: appState.selectedScope === 'project'
      ,config_paths: appState.configPaths
    });
    appState.scanResult = res;
    renderStatusBanner(res);
    renderTeamGrid(res);
  } catch (err) {
    showToast(`扫描失败: ${err.message}`, 'error');
  }
}

async function chooseConfigPath() {
  try {
    const path = await callRust('select_file', { filter_ext: 'jsonc' });
    if (path) {
      const input = document.getElementById('config-path-input');
      if (input) input.value = path;
      addConfigPath();
    }
  } catch (err) {
    showToast(`选择配置文件失败: ${err.message}`, 'error');
  }
}

function addConfigPath() {
  const input = document.getElementById('config-path-input');
  const value = input ? input.value.trim() : '';
  if (!value) {
    showToast('请先输入或选择 opencode.json / opencode.jsonc 路径', 'warning');
    return;
  }
  if (!/\\.(json|jsonc)$/i.test(value)) {
    showToast('配置路径必须指向 opencode.json 或 opencode.jsonc', 'warning');
    return;
  }
  if (!appState.configPaths.includes(value)) appState.configPaths.push(value);
  input.value = '';
  renderConfigPathList();
}

function renderConfigPathList() {
  const list = document.getElementById('config-path-list');
  if (!list) return;
  list.textContent = '';
  if (appState.configPaths.length === 0) {
    list.textContent = '未指定时，使用平台默认配置目录。';
    return;
  }
  appState.configPaths.forEach(path => {
    const chip = document.createElement('span');
    chip.className = 'config-path-chip';
    chip.textContent = path;
    list.appendChild(chip);
  });
}

async function confirmScan() {
  appState.scanConfirmed = true;
  const consent = document.getElementById('consent-screen');
  if (consent) consent.remove();
  await loadSavedAppConfig();
  refreshState();
}

async function chooseProjectFolder() {
  try {
    const folder = await callRust('select_folder');
    if (folder) {
      appState.projectPath = folder;
      showToast(`已选择项目目录: ${folder}`, 'success');
      refreshState();
    }
  } catch (err) {
    showToast(`选择目录失败: ${err.message}`, 'error');
  }
}

function renderStatusBanner(scan) {
  const banner = document.getElementById('banner-section');
  const icon = document.getElementById('banner-icon');
  const title = document.getElementById('banner-title');
  const desc = document.getElementById('banner-desc');
  const btnWizard = document.getElementById('btn-install-wizard');

  if (!banner || !icon || !title || !desc || !btnWizard) return;

  banner.className = 'banner-card';

  if (!scan || !scan.team_status) return;

  const st = scan.team_status;
  if (st.is_installed_complete) {
    banner.classList.add('complete');
    icon.textContent = '✅';
    title.textContent = `Rapid Dev Team 状态完整 (${st.installed_count}/${st.total_expected})`;
    desc.textContent = `配置文件位于: ${scan.opencode_dir} | 检测到 ${scan.models.length} 个可用模型定义`;
    btnWizard.style.display = 'none';
  } else if (st.is_installed_partial) {
    banner.classList.add('partial');
    icon.textContent = '⚠️';
    title.textContent = `Rapid Dev Team 部分就绪 (${st.installed_count}/${st.total_expected})`;
    desc.textContent = `缺少: ${st.missing_agents.join(', ')}。请前往安装向导补齐。`;
    btnWizard.style.display = 'inline-flex';
    btnWizard.textContent = '📦 补齐缺失 Agent';
  } else {
    banner.classList.add('uninstalled');
    icon.textContent = '❌';
    title.textContent = '未检测到 Rapid Dev Team 安装';
    desc.textContent = `当前 ${scan.target_scope === 'global' ? '全局' : '项目'} 目录尚未配置 Rapid Team 架构。`;
    btnWizard.style.display = 'inline-flex';
    btnWizard.textContent = '📦 一键安装 Rapid Team';
  }
}

function renderTeamGrid(scan) {
  const container = document.getElementById('team-grid');
  if (!container) return;
  container.innerHTML = '';

  appState.modelIndex = (scan.models || []).map(model => ({
    ...model,
    searchText: normalizeSearchText(model.id)
  }));

  const expectedAgents = [
    { name: 'rapid-dev-team', title: 'rapid-dev-team (队长/协调调度)', mode: 'primary', desc: '总协调调度，负责分发工作流及审核交付' },
    { name: 'rapid-scout', title: 'rapid-scout (侦察兵)', mode: 'subagent', desc: '负责项目侦察、依赖扫描、信息整理' },
    { name: 'rapid-builder-glm-zhipu', title: 'rapid-builder-glm-zhipu (GLM 智谱)', mode: 'subagent', desc: '基于智谱 GLM 模型的代码实现者' },
    { name: 'rapid-builder-glm-go', title: 'rapid-builder-glm-go (GLM Coding)', mode: 'subagent', desc: '基于 GLM Coding 优化实现者' },
    { name: 'rapid-builder-deepseek-go', title: 'rapid-builder-deepseek-go (DeepSeek Go)', mode: 'subagent', desc: '基于 DeepSeek 模型的高并发快速实现' },
    { name: 'rapid-builder-deepseek-sensenova', title: 'rapid-builder-deepseek-sensenova (商汤日日新)', mode: 'subagent', desc: '基于 SenseNova 体系的构建与优化' },
    { name: 'rapid-ui', title: 'rapid-ui (前端视觉与交互)', mode: 'subagent', desc: '负责页面 HTML/CSS/JS、设计系统与技术验证' },
    { name: 'rapid-reviewer', title: 'rapid-reviewer (代码评审与质检)', mode: 'subagent', desc: '严格检查代码规范、安全、性能与测试' },
    { name: 'rapid-architect', title: 'rapid-architect (架构设计与方案)', mode: 'subagent', desc: '负责方案设计、模块解耦与架构把关' }
  ];

  const agentMap = {};
  if (scan && scan.agents) {
    scan.agents.forEach(a => { agentMap[a.name] = a; });
  }

  expectedAgents.forEach(exp => {
    const existing = agentMap[exp.name];
    const card = document.createElement('div');
    card.className = 'agent-card';

    const currentModel = existing ? (existing.current_model || '(未指定)') : '(未安装)';
    const selectedModel = appState.selectedModels[exp.name] || (existing ? existing.current_model || '' : '');
    const sourceLabel = existing ? (existing.source === 'project' ? ' [项目]' : ' [全局]') : '';

    card.innerHTML = `
      <div class="agent-card-header">
        <div class="agent-title-box">
          <h3>${escapeHtml(exp.title)}</h3>
          <div class="agent-meta">
            <span>当前模型: <strong style="color:#60a5fa">${escapeHtml(currentModel)}</strong>${escapeHtml(sourceLabel)}</span>
            <span class="path">${escapeHtml(existing ? existing.full_path : '(尚未创建文件)')}</span>
          </div>
        </div>
        <span class="agent-badge ${exp.mode === 'primary' ? 'primary' : ''}">${escapeHtml(exp.mode)}</span>
      </div>
      <p style="font-size:12px; color:var(--text-muted);">${escapeHtml(exp.desc)}</p>
      <div class="model-selector-row">
        <label for="model-input-${escapeHtml(exp.name)}">设定/分配模型:</label>
        <div class="model-combobox">
          <input id="model-input-${escapeHtml(exp.name)}" type="search" class="model-select" data-agent="${escapeHtml(exp.name)}" value="${escapeHtml(selectedModel)}" placeholder="输入模型名称进行搜索" autocomplete="off" />
        </div>
      </div>
    `;

    const selectEl = card.querySelector('.model-select');
    selectEl.addEventListener('focus', () => openModelPicker(selectEl, exp.name));
    selectEl.addEventListener('input', () => {
      appState.selectedModels[exp.name] = selectEl.value.trim();
      openModelPicker(selectEl, exp.name);
    });
    selectEl.addEventListener('blur', scheduleCloseModelPicker);

    container.appendChild(card);
  });
}

function openModelPicker(input, agentName) {
  const panel = document.getElementById('shared-model-options');
  if (!panel) return;
  clearTimeout(appState.modelPickerTimer);
  appState.activeModelInput = input;
  appState.activeModelAgent = agentName;
  const rect = input.getBoundingClientRect();
  panel.style.left = `${Math.max(8, rect.left)}px`;
  panel.style.top = `${rect.bottom + 4}px`;
  panel.style.width = `${rect.width}px`;
  panel.classList.add('open');
  renderSharedModelOptions(input.value);
}

function renderSharedModelOptions(query) {
  const panel = document.getElementById('shared-model-options');
  if (!panel) return;
  const tokens = parseSearchTokens(query);
  const matches = appState.modelIndex.filter(model => {
    return tokens.every(token => {
      if (model.searchText.includes(token)) return true;
      let cursor = 0;
      for (const character of token) {
        cursor = model.searchText.indexOf(character, cursor);
        if (cursor === -1) return false;
        cursor += 1;
      }
      return true;
    });
  }).slice(0, 100);
  const groups = {};
  matches.forEach(model => (groups[model.provider] ||= []).push(model));
  panel.textContent = '';
  Object.entries(groups).forEach(([provider, models]) => {
    const heading = document.createElement('div');
    heading.className = 'model-provider-label';
    heading.textContent = `Provider: ${provider}`;
    panel.appendChild(heading);
    models.forEach(model => {
      const option = document.createElement('button');
      option.type = 'button';
      option.className = 'model-option';
      option.dataset.model = model.id;
      option.textContent = model.id;
      if (appState.activeModelInput && appState.activeModelInput.value === model.id) option.classList.add('selected');
      panel.appendChild(option);
    });
  });
  if (!matches.length) {
    const empty = document.createElement('span');
    empty.className = 'model-empty';
    empty.textContent = '没有匹配的模型';
    panel.appendChild(empty);
  }
}

function scheduleCloseModelPicker() {
  clearTimeout(appState.modelPickerTimer);
  appState.modelPickerTimer = setTimeout(() => {
    document.getElementById('shared-model-options')?.classList.remove('open');
  }, 150);
}

function filterModelOptions(query) {
  const q = query.trim();
  document.querySelectorAll('.agent-card').forEach(card => {
    const modelMatch = appState.modelIndex.some(model => fuzzyMatch(model.id, q));
    card.hidden = Boolean(q && !fuzzyMatch(card.textContent, q) && !modelMatch);
  });
  if (appState.activeModelInput) renderSharedModelOptions(appState.activeModelInput.value);
}

function scheduleModelFilter(query) {
  if (appState.searchFrame) cancelAnimationFrame(appState.searchFrame);
  appState.searchFrame = requestAnimationFrame(() => {
    appState.searchFrame = null;
    filterModelOptions(query);
  });
}

function autoMatchModels() {
  if (!appState.scanResult || !appState.scanResult.models) return;
  const models = appState.scanResult.models.map(m => m.id);
  if (models.length === 0) {
    showToast('未在配置中检测到已注册模型，请先配置 opencode.json/jsonc provider', 'warning');
    return;
  }

  // Smart heuristic matching
  const findBest = (patterns) => {
    for (const p of patterns) {
      const found = models.find(m => m.toLowerCase().includes(p.toLowerCase()));
      if (found) return found;
    }
    return models[0];
  };

  const mapping = {
    'rapid-dev-team': findBest(['gemini-3.7-flash', 'claude-3-7', 'gpt-4o', 'deepseek']),
    'rapid-scout': findBest(['gemini-2.5-flash', 'glm-4-flash', 'mini', 'flash', 'deepseek']),
    'rapid-builder-glm-zhipu': findBest(['glm-4', 'glm', 'deepseek']),
    'rapid-builder-glm-go': findBest(['glm-4', 'glm', 'deepseek']),
    'rapid-builder-deepseek-go': findBest(['deepseek-v3', 'deepseek', 'glm']),
    'rapid-builder-deepseek-sensenova': findBest(['sensenova', 'deepseek', 'glm']),
    'rapid-ui': findBest(['gemini-3.7-flash', 'gpt-4o', 'claude-3-7', 'glm-4']),
    'rapid-reviewer': findBest(['claude-3-7', 'gpt-4o', 'deepseek-reasoner', 'glm-4']),
    'rapid-architect': findBest(['claude-3-7', 'gpt-4o', 'gemini-3.7', 'deepseek'])
  };

  for (const [agent, model] of Object.entries(mapping)) {
    appState.selectedModels[agent] = model;
  }

  renderTeamGrid(appState.scanResult);
  showToast('已根据可用模型自动智能匹配！点击「预览修改」查看 Diff', 'success');
}

async function previewPlan() {
  if (!appState.scanResult) return;

  const realChangeItems = [];
  const agentMap = {};
  if (appState.scanResult.agents) {
    appState.scanResult.agents.forEach(a => { agentMap[a.name] = a; });
  }

  for (const [agentName, newModelRaw] of Object.entries(appState.selectedModels)) {
    const newModel = (newModelRaw || '').trim();
    if (!newModel) continue;

    const existing = agentMap[agentName];
    if (existing) {
      const orig = (existing.current_model || '').trim();
      if (orig !== newModel) {
        realChangeItems.push({
          agent_name: agentName,
          file_path: existing.full_path,
          original_model: existing.current_model || null,
          new_model: newModel,
          expected_mtime: existing.mtime,
          expected_hash: existing.content_hash || null
        });
      }
    }
  }

  if (realChangeItems.length === 0) {
    showToast('当前没有检测到任何模型配置变更', 'info');
    return;
  }

  try {
    const plan = await callRust('generate_change_plan', {
      base_opencode_dir: appState.scanResult.opencode_dir,
      items: realChangeItems
    });

    if (!plan.has_changes || plan.changes.length === 0) {
      showToast('当前没有检测到任何实际变更', 'info');
      return;
    }

    appState.currentPlan = { plan, items: realChangeItems };
    renderDiffModal(plan);
  } catch (err) {
    showToast(`生成变更计划失败: ${err.message}`, 'error');
  }
}

function renderDiffModal(plan) {
  const list = document.getElementById('diff-list');
  if (!list) return;
  list.innerHTML = '';

  const backupNotice = document.getElementById('modal-backup-plan');
  if (backupNotice) {
    backupNotice.textContent = `备份计划目录: ${plan.planned_backup_dir} (将完整保留相对路径)`;
  }

  plan.changes.forEach(c => {
    const item = document.createElement('div');
    item.className = 'diff-card';
    item.innerHTML = `
      <div class="diff-card-header">
        <span>🤖 ${escapeHtml(c.agent_name)} (<code style="color:#94a3b8">${escapeHtml(c.file_name)}</code>)</span>
        <span style="color:#10b981">→ ${escapeHtml(c.new_model)}</span>
      </div>
      <div style="font-size:11px; color:#64748b; margin-bottom:4px;">
        完整路径: <code>${escapeHtml(c.file_path)}</code> | 相对备份: <code>${escapeHtml(c.backup_rel_path)}</code>
      </div>
      <div class="diff-code">${escapeHtml(c.diff_preview)}</div>
    `;
    list.appendChild(item);
  });

  const modal = document.getElementById('diff-modal');
  if (modal) modal.style.display = 'flex';
}

function copyCurrentDiff() {
  if (!appState.currentPlan || !appState.currentPlan.plan) {
    showToast('无可用 Diff 数据', 'warning');
    return;
  }
  const { plan } = appState.currentPlan;
  let formatted = `# Rapid Agent Team Model Changes Plan\nBackup Directory: ${plan.planned_backup_dir}\n\n`;
  plan.changes.forEach(c => {
    formatted += `## Agent: ${c.agent_name} (${c.file_name})\n`;
    formatted += `Path: ${c.file_path}\n`;
    formatted += `Target Model: ${c.new_model}\n`;
    formatted += `\`\`\`diff\n${c.diff_preview}\n\`\`\`\n\n`;
  });
  copyToClipboard(formatted, '已复制 Diff 完整内容到剪贴板');
}

function closeModal() {
  const modal = document.getElementById('diff-modal');
  if (modal) modal.style.display = 'none';
}

async function confirmApply() {
  if (!appState.currentPlan || !appState.currentPlan.items || appState.currentPlan.items.length === 0) {
    showToast('没有可应用的有效变更', 'warning');
    closeModal();
    return;
  }

  const { items } = appState.currentPlan;
  const sanitizedItems = items.filter(it => it.new_model && it.new_model.trim().length > 0);
  if (sanitizedItems.length === 0) {
    showToast('变更列表为空', 'warning');
    closeModal();
    return;
  }

  try {
    const res = await callRust('apply_model_changes', {
      base_opencode_dir: appState.scanResult.opencode_dir,
      items: sanitizedItems
    });

    closeModal();
    showToast(res.message, 'success');
    appState.currentPlan = null;
    refreshState();
  } catch (err) {
    showToast(`应用修改失败: ${err.message}`, 'error');
  }
}

// Installation Handlers
async function installFromZip() {
  try {
    const zipPath = await callRust('select_file', { filter_ext: 'zip' });
    if (!zipPath) return;

    const plan = await callRust('generate_zip_install_plan', {
      zip_path: zipPath,
      target_opencode_dir: appState.scanResult.opencode_dir
    });

    const confirmMsg = `准备安装 Rapid Dev Team (${plan.total_files} 个文件，将覆盖 ${plan.overwrite_count} 个现有文件并自动备份)。确认继续？`;
    if (!confirm(confirmMsg)) {
      return;
    }

    const res = await callRust('install_from_zip', {
      zip_path: zipPath,
      target_opencode_dir: appState.scanResult.opencode_dir
    });

    showToast(res.message, 'success');
    refreshState();
  } catch (err) {
    showToast(`ZIP 安装失败: ${err.message}`, 'error');
  }
}

async function installFromDir() {
  try {
    const dirPath = await callRust('select_folder');
    if (!dirPath) return;

    // 与 ZIP 流程保持一致：安装前展示确认。install_from_local_dir 会先完整校验
    // team.config.json（名称/版本/rapid-* 清单），非 Rapid Team 包会直接报错，不会静默跳过。
    const confirmMsg = `将从本地文件夹安装 Rapid Dev Team：\n\n${escapeHtml(dirPath)}\n\n` +
      `安装前会自动校验 team.config.json 清单；已存在的目标文件将自动备份，异常时事务回滚。确认继续？`;
    if (!confirm(confirmMsg)) {
      return;
    }

    const res = await callRust('install_from_local_dir', {
      source_dir: dirPath,
      target_opencode_dir: appState.scanResult.opencode_dir
    });

    showToast(res.message, 'success');
    refreshState();
  } catch (err) {
    showToast(`目录安装失败: ${err.message}`, 'error');
  }
}

async function fetchRelease() {
  const infoBox = document.getElementById('release-info-box');
  try {
    showToast('正在获取 GitHub 最新版本...', 'info');
    const proxy = getProxyConfig();
    const rel = await callRust('fetch_latest_release', {
      repo: 'VastNext/opencode-rapid-agent-team',
      proxy
    });
    appState.latestRelease = rel;

    if (infoBox) {
      infoBox.innerHTML = `
        最新版本: <strong>${escapeHtml(rel.tag_name)}</strong> (${escapeHtml(rel.name)})<br/>
        所属仓库: <code>${escapeHtml(rel.repo)}</code> | 平台: <code>${escapeHtml(rel.current_platform)}</code><br/>
        ${rel.has_platform_asset ? `专属直链: <span style="color:#60a5fa">${escapeHtml(rel.direct_asset_name || rel.direct_asset_url)}</span>` : '<span style="color:#f59e0b">已匹配官方源码/Release压缩包</span>'}
      `;
    }

    const btnDownload = document.getElementById('btn-download-install-release');
    if (btnDownload) btnDownload.style.display = 'inline-flex';
    const btnCopyRel = document.getElementById('btn-copy-release-link');
    if (btnCopyRel) btnCopyRel.style.display = 'inline-flex';
    const btnCopySha = document.getElementById('btn-copy-sha256-link');
    if (btnCopySha) btnCopySha.style.display = rel.sha256_url ? 'inline-flex' : 'none';

    const pageBtn = document.getElementById('btn-open-release-page');
    if (pageBtn) {
      pageBtn.style.display = 'inline-flex';
      pageBtn.href = rel.html_url.startsWith('https://') ? rel.html_url : '#';
    }

    showToast(`获取到最新版本: ${rel.tag_name}`, 'success');
  } catch (err) {
    if (infoBox) {
      infoBox.innerHTML = `
        <span style="color:#ef4444">❌ 在线获取失败: ${escapeHtml(err.message)}</span><br/>
        <span style="color:var(--text-muted); font-size:12px;">提示：可在左侧使用「选项 A：从本地 ZIP 安装」或「选项 B：从本地文件夹安装」直接部署。</span>
      `;
    }
    showToast(`获取 Release 失败: ${err.message}`, 'error');
  }
}

async function downloadAndInstallRelease() {
  if (!appState.latestRelease) return;
  const url = appState.latestRelease.direct_asset_url || appState.latestRelease.zipball_url;

  try {
    showToast('正在下载并安全解压安装...', 'info');
    const proxy = getProxyConfig();
    const res = await callRust('download_and_install_release', {
      url,
      target_opencode_dir: appState.scanResult.opencode_dir,
      proxy
    });
    showToast(res.message, 'success');
    refreshState();
  } catch (err) {
    const infoBox = document.getElementById('release-info-box');
    if (infoBox) {
      infoBox.innerHTML += `<br/><span style="color:#ef4444; font-size:12px;">⚠️ 下载失败: ${escapeHtml(err.message)}。请尝试使用本地 ZIP 安装或配置代理。</span>`;
    }
    showToast(`下载安装失败: ${err.message}`, 'error');
  }
}

function copyReleaseLink() {
  if (!appState.latestRelease) return;
  const url = appState.latestRelease.direct_asset_url || appState.latestRelease.zipball_url;
  copyToClipboard(url, '已复制下载直链到剪贴板');
}

function copySha256Link() {
  if (!appState.latestRelease || !appState.latestRelease.sha256_url) return;
  copyToClipboard(appState.latestRelease.sha256_url, '已复制 SHA-256 校验文件地址');
}

// Proxy
function getProxyConfig() {
  const enabledEl = document.getElementById('proxy-enabled');
  const enabled = enabledEl ? enabledEl.checked : true;
  const urlEl = document.getElementById('proxy-url');
  const proxy_url = urlEl ? urlEl.value.trim() : 'http://127.0.0.1:7890';
  return { enabled, proxy_url };
}

async function testProxy() {
  const proxy = getProxyConfig();
  const box = document.getElementById('proxy-test-result');
  if (!box) return;
  box.style.display = 'block';
  box.style.color = 'var(--text-muted)';
  box.textContent = '正在测试 GitHub 连通性...';

  try {
    const res = await callRust('test_proxy_connection', { proxy });
    if (res.ok) {
      box.style.color = 'var(--success)';
      box.textContent = `✅ 连通测试通过！状态码: ${res.status_code}, 耗时: ${res.latency_ms} ms`;
    } else {
      box.style.color = 'var(--danger)';
      box.textContent = `❌ 测试未通过: ${res.message}`;
    }
  } catch (err) {
    box.style.color = 'var(--danger)';
    box.textContent = `❌ 测试执行异常: ${err.message}`;
  }
}

async function saveProxyConfig() {
  const proxy = getProxyConfig();
  appState.proxyConfig = proxy;
  try {
    await callRust('save_app_config', { proxy });
    showToast('代理设置已持久化保存至应用配置', 'success');
  } catch (err) {
    showToast(`保存代理设置失败: ${err.message}`, 'error');
  }
}
