import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { choose, request } from './api';
import { Button, messageOf } from './components';
import { ProviderReadinessResult, ReadinessButton, useProviderReadiness } from './ProviderReadiness';
import { actionBlockReason, currentExecutionPolicy, subscribeExecutionPolicy } from './executionPolicy';

type Props = { projectId: string; path: string; writable: boolean; update: (key: string, value: string) => void };

type Login = { state: 'idle' | 'starting' | 'authorizing' | 'authenticated' | 'failed' | 'cancelled' | 'logging_out' | 'logged_out'; message?: string; verificationUrl?: string; userCode?: string };
function loginResult(value: unknown): Login {
  const state = value as Login;
  if (!state || !['idle', 'starting', 'authorizing', 'authenticated', 'failed', 'cancelled', 'logging_out', 'logged_out'].includes(state.state)
      || state.message !== undefined && typeof state.message !== 'string'
      || state.verificationUrl !== undefined && typeof state.verificationUrl !== 'string'
      || state.userCode !== undefined && typeof state.userCode !== 'string') throw new Error('即梦授权状态无效，请重新检查。');
  return state;
}

/** One main connection: explicit OAuth + bounded progress + the same readiness card. */
export function DreaminaCanvasReadiness({ projectId, path, writable, update }: Props) {
  const policy = useSyncExternalStore(subscribeExecutionPolicy, currentExecutionPolicy, currentExecutionPolicy);
  const oauthAllowed = policy.canvasOAuth?.enabled === true && writable;
  const check = useProviderReadiness(projectId, 'dreamina_canvas', path, writable);
  const scope = JSON.stringify([projectId, path, writable]);
  const currentScope = useRef(scope), sequence = useRef(0), mounted = useRef(true), picking = useRef(false);
  if (currentScope.current !== scope) { currentScope.current = scope; sequence.current++; }
  const [selecting, setSelecting] = useState(false), [pickerError, setPickerError] = useState({ scope, message: '' });
  const [loginState, setLoginState] = useState<{ scope: string; value: Login }>({ scope, value: { state: 'idle' } });
  const login = loginState.scope === scope ? loginState.value : { state: 'idle' } as Login;
  const [confirmForce, setConfirmForce] = useState(false), [confirmLogout, setConfirmLogout] = useState(false);
  const loginBusy = login.state === 'starting' || login.state === 'authorizing' || login.state === 'logging_out';
  const authAttempt = useRef(0);
  const checkRef = useRef(check.check); checkRef.current = check.check;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; sequence.current++; }; }, []);

  // A native desktop reconnects to its existing session when the settings page
  // is reopened. Isolated probes and unapproved environments never do this.
  useEffect(() => {
    const attempt = ++authAttempt.current;
    setConfirmForce(false); setConfirmLogout(false);
    if (!oauthAllowed) return;
    let cancelled = false;
    void request<unknown>({ action: 'canvas.loginStatus', projectId, settings: { dreaminaCanvasPath: path } }).then(value => {
      if (cancelled || !mounted.current || currentScope.current !== scope || authAttempt.current !== attempt) return;
      const state = loginResult(value);
      setLoginState({ scope, value: state });
      if (!['starting', 'authorizing', 'authenticated', 'logging_out', 'logged_out'].includes(state.state)) void checkRef.current();
    }).catch(() => { if (!cancelled && mounted.current && currentScope.current === scope && authAttempt.current === attempt) void checkRef.current(); });
    return () => { cancelled = true; };
  }, [scope, oauthAllowed, projectId, path]);

  useEffect(() => {
    if (!oauthAllowed || !loginBusy) return;
    let cancelled = false;
    const attempt = authAttempt.current;
    const timer = setTimeout(() => {
      void request<unknown>({ action: 'canvas.loginStatus', projectId, settings: { dreaminaCanvasPath: path } }).then(value => {
        if (!cancelled && mounted.current && currentScope.current === scope && authAttempt.current === attempt) setLoginState({ scope, value: loginResult(value) });
      }).catch(reason => {
        if (!cancelled && mounted.current && currentScope.current === scope && authAttempt.current === attempt) setLoginState({ scope, value: { state: 'failed', message: messageOf(reason) } });
      });
    }, 1000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [loginState, loginBusy, scope, oauthAllowed, projectId, path]);

  useEffect(() => { if (oauthAllowed && (login.state === 'authenticated' || login.state === 'logged_out')) void checkRef.current(); }, [oauthAllowed, login.state, scope]);

  async function authorize(force = false) {
    if (!oauthAllowed || loginBusy || check.busy || selecting || actionBlockReason('canvas.login')) return;
    const checks = check.result?.details.checks;
    if (!force && checks && typeof checks === 'object' && Object.values(checks).some(item => item && typeof item === 'object' && 'code' in item && item.code === 'CANVAS_OAUTH_SCOPE_INSUFFICIENT')) {
      setConfirmForce(true); return;
    }
    const attempt = ++authAttempt.current;
    check.reset();
    setConfirmForce(false); setLoginState({ scope, value: { state: 'starting', message: '正在连接即梦…' } });
    try {
      const value = await request<unknown>({ action: 'canvas.login', projectId, settings: { dreaminaCanvasPath: path }, ...(force ? { force: true, confirmedForce: true } : {}) });
      if (mounted.current && currentScope.current === scope && attempt === authAttempt.current) setLoginState({ scope, value: loginResult(value) });
    } catch (reason) {
      if (mounted.current && currentScope.current === scope && attempt === authAttempt.current) setLoginState({ scope, value: { state: 'failed', message: messageOf(reason) } });
    }
  }
  async function sessionAction(action: 'canvas.cancelLogin' | 'canvas.openAuthorization') {
    if (!oauthAllowed || actionBlockReason(action)) return;
    const attempt = ++authAttempt.current;
    try {
      const value = await request<unknown>({ action, projectId, settings: { dreaminaCanvasPath: path } });
      if (mounted.current && currentScope.current === scope && attempt === authAttempt.current) setLoginState({ scope, value: loginResult(value) });
    } catch (reason) {
      if (mounted.current && currentScope.current === scope && attempt === authAttempt.current) setLoginState({ scope, value: { state: 'failed', message: messageOf(reason) } });
    }
  }

  async function logout() {
    if (!oauthAllowed || loginBusy || check.busy || actionBlockReason('canvas.logout')) return;
    const attempt = ++authAttempt.current;
    setConfirmLogout(false); check.reset();
    setLoginState({ scope, value: { state: 'logging_out', message: '正在退出登录…' } });
    try {
      const value = await request<unknown>({ action: 'canvas.logout', projectId, settings: { dreaminaCanvasPath: path }, confirmedLogout: true });
      if (mounted.current && currentScope.current === scope && attempt === authAttempt.current) setLoginState({ scope, value: loginResult(value) });
    } catch (reason) {
      if (mounted.current && currentScope.current === scope && attempt === authAttempt.current) setLoginState({ scope, value: { state: 'failed', message: messageOf(reason) } });
    }
  }

  async function selectExecutable() {
    if (!writable || !mounted.current || picking.current || currentScope.current !== scope) return;
    const attempt = ++sequence.current;
    picking.current = true; setSelecting(true); setPickerError({ scope, message: '' });
    try {
      const paths = await choose('file');
      if (mounted.current && attempt === sequence.current && currentScope.current === scope && paths[0]) update('dreaminaCanvasPath', paths[0]);
    } catch (reason) {
      if (mounted.current && attempt === sequence.current && currentScope.current === scope) setPickerError({ scope, message: messageOf(reason) });
    } finally {
      picking.current = false;
      if (mounted.current) setSelecting(false);
    }
  }

  const account = check.result?.details.accountQuery as { state?: string; isVip?: boolean; vipLevel?: string } | undefined;
  const hasLoginRecord = check.result?.authentication.state === 'authenticated';
  const identityVerified = hasLoginRecord && check.result?.authentication.identityVerified === true;
  const verified = identityVerified && account?.state === 'available';
  const membership = !verified || account?.isVip === undefined ? '尚未核验' : account.isVip ? `会员${account.vipLevel ? `（${account.vipLevel}）` : ''}` : '非会员';
  return <section className="surface dreamina-canvas-readiness" data-readiness-card="dreamina_canvas">
    <h3>即梦 Canvas 账户与连接</h3>
    <p className="muted">点击 OAuth，在即梦官方网页确认后自动连接。授权失效时可在这里重新连接。</p>
    <strong role="status">{login.state === 'logging_out' ? '正在退出登录…' : loginBusy ? '等待官方授权完成' : check.busy ? '正在核验连接…' : identityVerified && check.result?.catalog.state === 'available' ? '已连接即梦' : check.result?.authentication.state === 'not_logged_in' ? '未登录' : check.result ? '尚未连接，请检查或重新授权' : '尚未核验连接'}</strong>
    <dl className="canvas-account-summary">
      <div><dt>登录状态</dt><dd>{loginBusy ? login.state === 'logging_out' ? '正在退出' : '等待授权' : check.busy ? '正在刷新' : identityVerified ? '已登录 · 身份已核验' : hasLoginRecord ? '本机有登录记录 · 身份待核验' : check.result?.authentication.state === 'not_logged_in' ? '未登录' : '尚未核验'}</dd></div>
      <div><dt>会员信息</dt><dd>{membership}</dd></div>
      <div><dt>账户额度</dt><dd>当前接口未提供</dd></div>
      <div><dt>可用模型目录</dt><dd>{check.result?.catalog.state === 'available' ? `${check.result.catalog.models.length} 个图片模型` : '尚未读取'}</dd></div>
    </dl>
    <p className="control-hint">当前 Canvas CLI 不提供总额度及会员／赠送／购买额度明细，无法显示余额。</p>
    <div className="actions">
      <Button executionAction="canvas.login" variant="primary" disabled={!oauthAllowed || loginBusy || check.busy || selecting} onClick={() => void authorize()}>OAuth 连接即梦</Button>
      <ReadinessButton check={check} writable={writable && !selecting && !loginBusy}>检查即梦 CLI、账户与模型</ReadinessButton>
      {loginBusy && login.state !== 'logging_out' && <><Button executionAction="canvas.openAuthorization" disabled={!oauthAllowed || login.state !== 'authorizing'} onClick={() => void sessionAction('canvas.openAuthorization')}>打开授权网页</Button><Button executionAction="canvas.cancelLogin" disabled={!oauthAllowed} onClick={() => void sessionAction('canvas.cancelLogin')}>取消等待</Button></>}
      {!loginBusy && <Button disabled={!oauthAllowed || check.busy} onClick={() => { setConfirmLogout(false); setConfirmForce(true); }}>重新登录</Button>}
      {!loginBusy && <Button executionAction="canvas.logout" disabled={!oauthAllowed || check.busy || !hasLoginRecord} onClick={() => { setConfirmForce(false); setConfirmLogout(true); }}>退出登录</Button>}
    </div>
    {confirmLogout && <div role="alert"><p>退出当前 Canvas CLI 的 default 配置登录。使用同一配置的其他程序也会受影响；项目和历史记录保留。</p><Button executionAction="canvas.logout" disabled={loginBusy || check.busy} onClick={() => void logout()}>确认退出登录</Button><Button onClick={() => setConfirmLogout(false)}>返回</Button></div>}
    {login.message && <p className={login.state === 'failed' ? 'warning' : 'control-hint'} role="status">{login.message}</p>}
    {login.state === 'authorizing' && login.userCode && <p>授权码：<code>{login.userCode}</code></p>}
    {confirmForce && <div role="alert"><p>重新授权会清除当前即梦配置的登录状态，并打开官方授权页。项目与历史任务保留。</p><Button variant="primary" disabled={loginBusy || check.busy} onClick={() => void authorize(true)}>确认重新授权</Button><Button onClick={() => setConfirmForce(false)}>返回</Button></div>}
    <details className="details"><summary>高级连接设置</summary><p className="control-hint">检查范围：default 配置 / cn 区域</p>
    <label>Canvas CLI 程序<div className="text-cli-path"><input data-canvas-path value={path} disabled={!writable || loginBusy} onChange={event => update('dreaminaCanvasPath', event.target.value)} placeholder="选择 dreamina-canvas，或填写可执行文件的完整路径"/><Button data-canvas-picker icon="source" disabled={!writable || selecting || loginBusy} onClick={() => void selectExecutable()}>{selecting ? '正在选择…' : '选择 Canvas 程序'}</Button></div></label></details>
    {pickerError.scope === scope && pickerError.message && <p className="warning" role="alert">{pickerError.message}</p>}
    <details className="details" data-connection-diagnostics><summary>查看连接详情与模型目录</summary><ProviderReadinessResult check={check}/></details>
    <p className="control-hint">连接和授权不会生成图片或视频，也不会消耗生成额度。图片、视频生成能力仍待接入。</p>
  </section>;
}
