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
  latestRelease: null
};

// RPC invoker to Rust backend via wry ipc
function callRust(action, payload = {}) {
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

    const message = JSON.stringify({
      action,
      callbackId,
      payload
    });

    if (window.ipc) {
      window.ipc.postMessage(message);
    } else {
      console.warn("IPC not available, running in mock/browser mode", action, payload);
      // Fallback mock for browser preview
      setTimeout(() => {
        resolve({ mock: true });
      }, 300);
    }
  });
}

function showToast(message, type = 'info') {
  const container = document.getElementById('toast-container');
  const toast = document.createElement('div');
  toast.className = `toast ${type}`;
  toast.textContent = message;
  container.appendChild(toast);
  setTimeout(() => {
    toast.remove();
  }, 4000);
}

// Initial Load
window.addEventListener('DOMContentLoaded', () => {
  setupEventHandlers();
  refreshState();
});

function setupEventHandlers() {
  // Tabs
  document.querySelectorAll('.tab-btn').forEach(btn => {
    btn.addEventListener('click', (e) => {
      document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
      document.querySelectorAll('.tab-pane').forEach(p => p.classList.remove('active'));
      btn.classList.add('active');
      const tabId = btn.getAttribute('data-tab');
      document.getElementById(tabId).classList.add('active');
    });
  });

  // Scope Toggle
  document.getElementById('scope-global').addEventListener('change', () => {
    appState.selectedScope = 'global';
    document.getElementById('btn-select-project').style.display = 'none';
    refreshState();
  });
  document.getElementById('scope-project').addEventListener('change', () => {
    appState.selectedScope = 'project';
    document.getElementById('btn-select-project').style.display = 'inline-flex';
    if (!appState.projectPath) {
      chooseProjectFolder();
    } else {
      refreshState();
    }
  });

  // Buttons
  document.getElementById('btn-refresh').addEventListener('click', refreshState);
  document.getElementById('btn-select-project').addEventListener('click', chooseProjectFolder);
  document.getElementById('btn-install-wizard').addEventListener('click', () => {
    document.querySelector('[data-tab="tab-install"]').click();
  });

  document.getElementById('model-filter-input').addEventListener('input', (e) => {
    filterModelOptions(e.target.value);
  });

  document.getElementById('btn-batch-apply-default').addEventListener('click', autoMatchModels);
  document.getElementById('btn-preview-plan').addEventListener('click', previewPlan);

  // Modal
  document.getElementById('btn-close-modal').addEventListener('click', closeModal);
  document.getElementById('btn-cancel-apply').addEventListener('click', closeModal);
  document.getElementById('btn-confirm-apply').addEventListener('click', confirmApply);

  // Install handlers
  document.getElementById('btn-install-zip').addEventListener('click', installFromZip);
  document.getElementById('btn-install-dir').addEventListener('click', installFromDir);
  document.getElementById('btn-fetch-release').addEventListener('click', fetchRelease);
  document.getElementById('btn-download-install-release').addEventListener('click', downloadAndInstallRelease);
  document.getElementById('btn-copy-release-link').addEventListener('click', copyReleaseLink);

  // Proxy
  document.getElementById('btn-test-proxy').addEventListener('click', testProxy);
  document.getElementById('btn-save-proxy').addEventListener('click', saveProxyConfig);
}

async function refreshState() {
  try {
    const res = await callRust('scan_environment', {
      project_path: appState.projectPath,
      use_project: appState.selectedScope === 'project'
    });
    appState.scanResult = res;
    renderStatusBanner(res);
    renderTeamGrid(res);
  } catch (err) {
    showToast(`扫描失败: ${err.message}`, 'error');
  }
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

  banner.className = 'banner-card';

  if (!scan.team_status) return;

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
  container.innerHTML = '';

  const expectedAgents = [
    { name: 'rapid-dev-team', title: 'rapid-dev-team (队长/协调调度)', mode: 'primary', desc: '总协调调度，负责分发工作流及审核交付' },
    { name: 'rapid-scout', title: 'rapid-scout (侦察兵)', mode: 'subagent', desc: '负责项目侦察、依赖扫描、信息整理' },
    { name: 'rapid-builder-glm-zhipu', title: 'rapid-builder-glm-zhipu (GLM 智谱)', mode: 'subagent', desc: '基于智谱 GLM 模型的代码实现者' },
    { name: 'rapid-builder-glm-go', title: 'rapid-builder-glm-go (GLM Coding)', mode: 'subagent', desc: '基于 GLM Coding 优化实现者' },
    { name: 'rapid-builder-deepseek-go', title: 'rapid-builder-deepseek-go (DeepSeek Go)', mode: 'subagent', desc: '基于 DeepSeek 模型的高并发快速实现' },
    { name: 'rapid-builder-deepseek-sensenova', title: 'rapid-builder-deepseek-sensenova (商汤日日新)', mode: 'subagent', desc: '基于 SenseNova 体系的构建与优化' },
    { name: 'rapid-ui', title: 'rapid-ui (前端视觉与交互)', mode: 'subagent', desc: '负责页面 HTML/CSS/JS、设计系统与截图验证' },
    { name: 'rapid-reviewer', title: 'rapid-reviewer (代码评审与质检)', mode: 'subagent', desc: '严格检查代码规范、安全、性能与测试' },
    { name: 'rapid-architect', title: 'rapid-architect (架构设计与方案)', mode: 'subagent', desc: '负责方案设计、模块解耦与架构把关' }
  ];

  const agentMap = {};
  scan.agents.forEach(a => { agentMap[a.name] = a; });

  expectedAgents.forEach(exp => {
    const existing = agentMap[exp.name];
    const card = document.createElement('div');
    card.className = 'agent-card';

    const isInstalled = !!existing;
    const currentModel = existing ? (existing.current_model || '(未指定)') : '(未安装)';
    const selectedModel = appState.selectedModels[exp.name] || (existing ? existing.current_model || '' : '');

    // Group models by provider
    const modelsByProvider = {};
    scan.models.forEach(m => {
      if (!modelsByProvider[m.provider]) modelsByProvider[m.provider] = [];
      modelsByProvider[m.provider].push(m);
    });

    let selectOptions = `<option value="">-- 选择或输入模型 --</option>`;
    for (const [provider, list] of Object.entries(modelsByProvider)) {
      selectOptions += `<optgroup label="Provider: ${provider}">`;
      list.forEach(m => {
        const isSel = m.id === selectedModel ? 'selected' : '';
        selectOptions += `<option value="${m.id}" ${isSel}>${m.id}</option>`;
      });
      selectOptions += `</optgroup>`;
    }

    card.innerHTML = `
      <div class="agent-card-header">
        <div class="agent-title-box">
          <h3>${exp.title}</h3>
          <div class="agent-meta">
            <span>当前模型: <strong style="color:#60a5fa">${currentModel}</strong></span>
            <span class="path">${existing ? existing.full_path : '(尚未创建文件)'}</span>
          </div>
        </div>
        <span class="agent-badge ${exp.mode === 'primary' ? 'primary' : ''}">${exp.mode}</span>
      </div>
      <p style="font-size:12px; color:var(--text-muted);">${exp.desc}</p>
      <div class="model-selector-row">
        <label>设定/分配模型:</label>
        <select class="model-select" data-agent="${exp.name}">
          ${selectOptions}
        </select>
      </div>
    `;

    const selectEl = card.querySelector('.model-select');
    selectEl.addEventListener('change', (e) => {
      appState.selectedModels[exp.name] = e.target.value;
    });

    container.appendChild(card);
  });
}

function filterModelOptions(query) {
  const q = query.trim().toLowerCase();
  document.querySelectorAll('.model-select').forEach(sel => {
    Array.from(sel.options).forEach(opt => {
      if (opt.value === '') return;
      const match = opt.text.toLowerCase().includes(q) || opt.value.toLowerCase().includes(q);
      opt.style.display = match || !q ? '' : 'none';
    });
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

  const items = [];
  const agentMap = {};
  appState.scanResult.agents.forEach(a => { agentMap[a.name] = a; });

  for (const [agentName, newModel] of Object.entries(appState.selectedModels)) {
    const existing = agentMap[agentName];
    if (existing) {
      items.push({
        agent_name: agentName,
        file_path: existing.full_path,
        original_model: existing.current_model || null,
        new_model: newModel,
        expected_mtime: existing.mtime
      });
    }
  }

  try {
    const plan = await callRust('generate_change_plan', { items });
    appState.currentPlan = { plan, items };

    if (!plan.has_changes) {
      showToast('当前没有检测到任何模型配置变更', 'info');
      return;
    }

    renderDiffModal(plan);
  } catch (err) {
    showToast(`生成变更计划失败: ${err.message}`, 'error');
  }
}

function renderDiffModal(plan) {
  const list = document.getElementById('diff-list');
  list.innerHTML = '';

  plan.changes.forEach(c => {
    const item = document.createElement('div');
    item.className = 'diff-card';
    item.innerHTML = `
      <div class="diff-card-header">
        <span>🤖 ${c.agent_name} (<code style="color:#94a3b8">${c.file_name}</code>)</span>
        <span style="color:#10b981">→ ${c.new_model}</span>
      </div>
      <div class="diff-code">${c.diff_preview}</div>
    `;
    list.appendChild(item);
  });

  document.getElementById('diff-modal').style.display = 'flex';
}

function closeModal() {
  document.getElementById('diff-modal').style.display = 'none';
}

async function confirmApply() {
  if (!appState.currentPlan) return;
  const { items } = appState.currentPlan;

  try {
    const res = await callRust('apply_model_changes', {
      base_opencode_dir: appState.scanResult.opencode_dir,
      items
    });

    closeModal();
    showToast(res.message, 'success');
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
  try {
    showToast('正在获取 GitHub 最新版本...', 'info');
    const proxy = getProxyConfig();
    const rel = await callRust('fetch_latest_release', {
      repo: 'VastNext/rapid-agent-team-config',
      proxy
    });
    appState.latestRelease = rel;

    document.getElementById('release-version-text').innerHTML = `
      最新版本: <strong>${rel.tag_name}</strong> (${rel.name})<br/>
      直链: <span style="color:#60a5fa">${rel.direct_asset_url || rel.zipball_url}</span>
    `;

    document.getElementById('btn-download-install-release').style.display = 'inline-flex';
    document.getElementById('btn-copy-release-link').style.display = 'inline-flex';
    const pageBtn = document.getElementById('btn-open-release-page');
    pageBtn.style.display = 'inline-flex';
    pageBtn.href = rel.html_url;

    showToast(`获取到最新版本: ${rel.tag_name}`, 'success');
  } catch (err) {
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
    showToast(`下载安装失败: ${err.message}`, 'error');
  }
}

function copyReleaseLink() {
  if (!appState.latestRelease) return;
  const url = appState.latestRelease.direct_asset_url || appState.latestRelease.zipball_url;
  navigator.clipboard.writeText(url);
  showToast('已复制下载直链到剪贴板', 'success');
}

// Proxy
function getProxyConfig() {
  const enabled = document.getElementById('proxy-enabled').checked;
  const proxy_url = document.getElementById('proxy-url').value.trim();
  return { enabled, proxy_url };
}

async function testProxy() {
  const proxy = getProxyConfig();
  const box = document.getElementById('proxy-test-result');
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

function saveProxyConfig() {
  appState.proxyConfig = getProxyConfig();
  showToast('代理配置已在应用局部生效', 'success');
}
