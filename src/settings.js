// 设置页：账号增删改、刷新间隔、开机自启
/* global window, document, confirm */
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let state = { accounts: [], usage: {}, selectedAccountId: null };
let editingId = null;

const $ = (id) => document.getElementById(id);

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}

function colorCls(p) {
  if (typeof p !== 'number') return 'c-gray';
  if (p <= 50) return 'c-green';
  if (p <= 80) return 'c-yellow';
  return 'c-red';
}

function toast(msg, isErr) {
  const t = $('toast');
  t.textContent = msg;
  t.classList.toggle('err', !!isErr);
  t.classList.add('show');
  clearTimeout(t._h);
  t._h = setTimeout(() => t.classList.remove('show'), 2600);
}

function renderList() {
  const box = $('account-list');
  box.textContent = '';
  const accs = state.accounts || [];
  if (!accs.length) {
    box.appendChild(el('p', 'hint', '还没有账号，点击下方按钮添加。'));
    return;
  }
  accs.forEach((a) => {
    const u = (state.usage || {})[a.id] || {};
    const p = typeof u.fiveHourUsed === 'number' ? u.fiveHourUsed : null;
    const row = el('div', 's-acc' + (a.id === state.selectedAccountId ? ' sel' : ''));
    const meta = `${a.type === 2 ? '团队' : '个人'} · 5h ${p === null ? '--' : Math.round(p) + '%'}`
      + (a.id === state.selectedAccountId ? ' · 菜单栏当前' : '');
    const be = el('button', '', '编辑');
    be.addEventListener('click', () => startEdit(a));
    const bd = el('button', 'danger', '删除');
    bd.addEventListener('click', async () => {
      if (!confirm(`删除账号「${a.name}」？将同时删除钥匙串中的 API Key。`)) return;
      try {
        await invoke('delete_account', { id: a.id });
        toast('已删除');
      } catch (e) {
        toast(`删除失败：${e}`, true);
      }
    });
    row.append(
      el('span', 'dot ' + colorCls(p)),
      el('span', 's-name', a.name),
      el('span', 's-meta', meta),
      be,
      bd
    );
    box.appendChild(row);
  });
}

function openForm(isNew) {
  editingId = isNew ? null : editingId;
  $('form-title').textContent = isNew ? '添加账号' : '编辑账号';
  $('f-key').required = isNew;
  $('f-key').placeholder = isNew
    ? '智谱开放平台 API Key'
    : '留空表示保持不变';
  $('account-form').hidden = false;
  $('btn-add').hidden = true;
  $('f-name').focus();
}

function closeForm() {
  $('account-form').hidden = true;
  $('account-form').reset();
  editingId = null;
  $('btn-add').hidden = false;
}

function startEdit(a) {
  editingId = a.id;
  $('f-name').value = a.name;
  $('f-key').value = '';
  $('f-org').value = a.org || '';
  $('f-project').value = a.project || '';
  $('f-type').value = String(a.type || 1);
  openForm(false);
}

function initGeneral() {
  const savedMin = String(state.refreshIntervalMin || 5);
  if ([...$('interval').options].some((o) => o.value === savedMin)) {
    $('interval').value = savedMin;
  }
  $('startup').checked = !!state.launchAtStartup;

  $('interval').addEventListener('change', async () => {
    try {
      await invoke('set_refresh_interval', { minutes: Number($('interval').value) });
      toast('已更新刷新间隔');
    } catch (e) {
      toast(`设置失败：${e}`, true);
    }
  });
  $('startup').addEventListener('change', async () => {
    const on = $('startup').checked;
    try {
      await invoke('set_launch_at_startup', { enabled: on });
      toast(on ? '已开启开机自启' : '已关闭开机自启');
    } catch (e) {
      $('startup').checked = !on;
      toast(`设置失败：${e}`, true);
    }
  });
}

async function init() {
  try {
    state = await invoke('get_state');
  } catch (e) {
    toast(`读取配置失败：${e}`, true);
    state = { accounts: [], usage: {}, selectedAccountId: null };
  }
  renderList();
  initGeneral();

  listen('state-changed', (e) => {
    state = e.payload;
    renderList();
    const savedMin = String(state.refreshIntervalMin || 5);
    if ([...$('interval').options].some((o) => o.value === savedMin)) {
      $('interval').value = savedMin;
    }
    $('startup').checked = !!state.launchAtStartup;
  });

  $('btn-add').addEventListener('click', () => {
    closeForm();
    openForm(true);
  });
  $('btn-cancel').addEventListener('click', closeForm);
  $('account-form').addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const name = $('f-name').value.trim();
    const key = $('f-key').value.trim();
    if (!name) return;
    if (!editingId && !key) {
      toast('新增账号必须填写 API Key', true);
      return;
    }
    try {
      await invoke('save_account', {
        id: editingId || null,
        name,
        apiKey: key || null,
        org: $('f-org').value.trim(),
        project: $('f-project').value.trim(),
        accountType: Number($('f-type').value),
      });
      toast('已保存，正在刷新额度…');
      closeForm();
    } catch (e) {
      toast(`保存失败：${e}`, true);
    }
  });
}

init();
