// 面板逻辑：渲染账号列表、切换选中账号、刷新动画、倒计时每 30s 本地重算
/* global window, document */
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// Windows 用不透明窗口（透明窗口在 WebView2 有渲染错位/命中失效问题），
// 内容铺满窗口、尺寸固定，不做运行时缩放
const IS_WIN = navigator.userAgent.includes('Windows');
if (IS_WIN) document.body.classList.add('win');

let state = { accounts: [], usage: {}, selectedAccountId: null, lastRefreshAt: null };

const $ = (id) => document.getElementById(id);

function pctOf(acc) {
  const u = (state.usage || {})[acc.id];
  return u && typeof u.fiveHourUsed === 'number' ? u.fiveHourUsed : null;
}

// 统一口径：已用 ≤50 绿，≤80 黄，>80 红，无数据灰
function colorCls(p) {
  if (p === null || p === undefined) return 'c-gray';
  if (p <= 50) return 'c-green';
  if (p <= 80) return 'c-yellow';
  return 'c-red';
}

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}

function formatCountdown(ts) {
  if (!ts) return '--';
  const remain = ts * 1000 - Date.now();
  if (remain <= 0) return '待刷新';
  const h = Math.floor(remain / 3600000);
  const m = Math.floor((remain % 3600000) / 60000);
  return h > 0 ? `${h}h ${m}m 后重置` : `${m}m 后重置`;
}

function timeAgo(ts) {
  if (!ts) return '--';
  const s = Math.floor((Date.now() - ts * 1000) / 1000);
  if (s < 60) return '刚刚';
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  return `${Math.floor(s / 3600)} 小时前`;
}

function accountNode(acc) {
  const u = (state.usage || {})[acc.id] || {};
  const p = pctOf(acc);
  const c = colorCls(p);
  const selected = acc.id === state.selectedAccountId;

  const row = el('div', 'account' + (selected ? ' selected' : ''));
  // 单击账号行：切换菜单栏显示的账号，面板不关闭
  row.addEventListener('click', () => {
    if (!selected) invoke('select_account', { id: acc.id }).catch(console.error);
  });

  const row1 = el('div', 'row1');
  row1.append(
    el('span', 'radio', selected ? '●' : '○'),
    el('span', 'name', acc.name),
    el('span', 'pct ' + c, p === null ? 'G·--' : `G·${Math.round(p)}%`)
  );

  const bar = el('div', 'bar');
  const fill = el('i', colorCls(p));
  fill.style.width = (p === null ? 0 : Math.min(100, p)) + '%';
  bar.appendChild(fill);

  const sub = el('div', 'sub');
  const cd = el('span', 'cd', '');
  sub.append(cd, document.createTextNode(' · '));
  if (u.error) {
    if (p === null) {
      sub.appendChild(el('span', 'err', '获取失败'));
    } else {
      sub.appendChild(
        el('span', 'weekly',
          typeof u.weeklyUsed === 'number' ? `周额度 已用 ${Math.round(u.weeklyUsed)}%` : '周额度 --')
      );
      sub.appendChild(el('span', 'err', '（缓存）'));
      row.title = `上次错误：${u.error}`;
    }
  } else {
    sub.appendChild(
      el('span', 'weekly',
        typeof u.weeklyUsed === 'number' ? `周额度 已用 ${Math.round(u.weeklyUsed)}%` : '周额度 --')
    );
  }
  updateCountdown(cd, p === null && u.error ? null : u.fiveHourResetAt);

  row.append(row1, bar, sub);
  return row;
}

function updateCountdown(node, resetAt) {
  node.textContent = resetAt ? formatCountdown(resetAt) : '--';
}

function emptyNode() {
  const box = el('div', 'empty');
  box.append(
    el('div', '', '还没有配置账号'),
    el('div', '', '在设置中添加 GLM Coding Plan 账号')
  );
  const btn = el('button', 'empty-btn', '去设置 →');
  btn.addEventListener('click', () => invoke('open_settings').catch(console.error));
  box.appendChild(btn);
  return box;
}

function render() {
  const list = $('accounts');
  list.textContent = '';
  const accs = state.accounts || [];
  if (!accs.length) {
    list.appendChild(emptyNode());
  } else {
    accs.forEach((acc, i) => {
      if (i > 0) list.appendChild(el('div', 'divider'));
      list.appendChild(accountNode(acc));
    });
  }
  updateFooter();
  requestAnimationFrame(fitHeight);
}

function updateFooter() {
  $('last-updated').textContent = `最后更新：${timeAgo(state.lastRefreshAt)}`;
  const n = (state.accounts || []).length;
  $('acc-count').textContent = n ? `共 ${n} 个账号` : '';
}

// 倒计时与"最后更新"每 30 秒本地重算，不重新请求接口
function tick() {
  document.querySelectorAll('.account').forEach((row, i) => {
    const acc = (state.accounts || [])[i];
    if (!acc) return;
    const u = (state.usage || {})[acc.id] || {};
    const cd = row.querySelector('.cd');
    if (cd) updateCountdown(cd, u.fiveHourResetAt);
  });
  updateFooter();
}

function fitHeight() {
  if (IS_WIN) return; // Windows 窗口尺寸固定 344×420
  const panel = $('panel');
  const rect = panel.getBoundingClientRect();
  const h = rect.height + 24; // 上下各 12px 留白
  invoke('resize_panel', { height: h }).catch(console.error);
}

function setSpin(on) {
  $('btn-refresh').classList.toggle('spinning', on);
}

async function doRefresh() {
  setSpin(true);
  try {
    state = await invoke('refresh_all');
    render();
  } catch (e) {
    console.error('refresh failed', e);
  } finally {
    setSpin(false);
  }
}

async function init() {
  try {
    state = await invoke('get_state');
  } catch (e) {
    console.error('get_state failed', e);
  }
  render();

  listen('state-changed', (e) => {
    state = e.payload;
    render();
    setSpin(false);
  });
  listen('refresh-started', () => setSpin(true));
  listen('tauri://focus', () => document.body.classList.add('open'));
  listen('tauri://blur', () => document.body.classList.remove('open'));
  // 兜底：若 focus 事件未送达（窗口已被系统聚焦等），也确保面板可见
  setTimeout(() => document.body.classList.add('open'), 400);

  $('btn-refresh').addEventListener('click', doRefresh);
  $('btn-settings').addEventListener('click', () => invoke('open_settings').catch(console.error));
  document.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape') invoke('hide_panel').catch(console.error);
  });

  setInterval(tick, 30000);
}

init();
