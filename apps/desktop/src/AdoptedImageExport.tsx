import { useEffect, useRef, useState } from 'react';
import { choose, request } from './api';
import { Button, messageOf, Modal } from './components';
import { imageScopeAssets } from './assetWorkflow';
import type { Workspace } from './types';

type ExportIssue = { assetId: string; name?: string; importName?: string; filename?: string | null; reason: string; message: string };
type ExportReceipt = {
  version: 'desktop-current-image-export/v1'; path: string; imageDirectory: string; manifestPath: string;
  status: 'complete' | 'partial' | 'no_ready_images'; scopeCount: number; count: number;
  missingCount: number; excludedCount: number; missing: ExportIssue[]; excluded: ExportIssue[];
  generationSubmissions: 0; videoSubmissions: 0;
};
export type AdoptedImageExportProps = { workspace: Workspace; writable: boolean; onNotice: (message: string) => void; onError: (message: string) => void };

function checkedReceipt(value: unknown): ExportReceipt {
  const receipt = value as ExportReceipt | null;
  const issuesValid = (items: unknown): items is ExportIssue[] => Array.isArray(items) && items.every(item => item && typeof item.assetId === 'string' && typeof item.reason === 'string' && typeof item.message === 'string'
    && ['name', 'importName'].every(key => item[key] === undefined || typeof item[key] === 'string')
    && (item.filename == null || typeof item.filename === 'string'));
  if (!receipt || receipt.version !== 'desktop-current-image-export/v1'
    || !['path', 'imageDirectory', 'manifestPath'].every(key => typeof receipt[key as keyof ExportReceipt] === 'string' && String(receipt[key as keyof ExportReceipt]).trim())
    || ![receipt.scopeCount, receipt.count, receipt.missingCount, receipt.excludedCount].every(count => Number.isSafeInteger(count) && count >= 0)
    || !issuesValid(receipt.missing) || !issuesValid(receipt.excluded)
    || receipt.missing.length !== receipt.missingCount || receipt.excluded.length !== receipt.excludedCount
    || receipt.count + receipt.missingCount + receipt.excludedCount !== receipt.scopeCount
    || receipt.generationSubmissions !== 0 || receipt.videoSubmissions !== 0) throw Error('导出回执不完整，请核对本次目录；未将其标记为导出完成。');
  const expectedStatus = receipt.missingCount === 0 && receipt.excludedCount === 0 ? 'complete' : receipt.count > 0 ? 'partial' : 'no_ready_images';
  if (receipt.status !== expectedStatus) throw Error('导出数量与完成状态不一致，请核对本次导出清单。');
  return receipt;
}
function resultMessage(receipt: ExportReceipt): string {
  if (receipt.status === 'complete') return `正式采用图片已全部导出，共 ${receipt.count} 张。`;
  if (receipt.status === 'no_ready_images') return '当前没有可导出的正式采用图，已保存待补清单。';
  return `已部分导出 ${receipt.count}/${receipt.scopeCount} 张；还有 ${receipt.missingCount} 项缺图或待复核${receipt.excludedCount ? `，${receipt.excludedCount} 项已排除` : ''}。`;
}

export function AdoptedImageExport({ workspace, writable, onNotice, onError }: AdoptedImageExportProps) {
  const [exporting, setExporting] = useState(false), [receipt, setReceipt] = useState<ExportReceipt | null>(null);
  const busy = useRef(false), active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const scopeCount = workspace.imageProductionScope?.assetIds.length ?? imageScopeAssets(workspace).length;
  async function exportImages() {
    if (!writable || !scopeCount || busy.current) return;
    busy.current = true; setExporting(true);
    const projectId = workspace.project.id;
    try {
      const [path] = await choose('directory');
      if (!path || !active.current) return;
      // The core snapshots the complete formal scope and adopted files itself.
      // List filters, selected checkboxes and reference-only IDs are not inputs.
      const result = checkedReceipt(await request<unknown>({ action: 'images.exportCurrent', projectId, path }));
      if (active.current) { setReceipt(result); onNotice(resultMessage(result)); }
    } catch (error) { if (active.current) onError(messageOf(error)); }
    finally { busy.current = false; if (active.current) setExporting(false); }
  }
  return <>
    <Button executionAction="images.exportCurrent" icon="download" disabled={!writable || !scopeCount || exporting} title="导出正式范围的当前采用图，沿用精确 PNG 文件名，不受当前列表筛选或勾选影响。" onClick={() => void exportImages()}>{exporting ? '正在导出图片…' : '导出已采用图片'}</Button>
    {receipt && <Modal title="已采用图片导出结果" onClose={() => setReceipt(null)}>
      <p role="status" className={receipt.status === 'complete' ? '' : 'warning'}>{resultMessage(receipt)}</p>
      <p>本次按正式图片范围导出，参考母版、候选图和历史图不会加入图片文件夹。已有图片、版本及采用记录保持原样。</p>
      <label>图片文件夹<code className="path-value">{receipt.imageDirectory}</code></label>
      {receipt.missing.length > 0 && <details className="details" open><summary>缺图或待处理 · {receipt.missingCount} 项</summary><ul>{receipt.missing.map(item => <li key={item.assetId}><strong>{item.name || item.importName || item.filename || item.assetId}</strong>：{item.message}{item.filename && <small> · {item.filename}</small>}</li>)}</ul></details>}
      {receipt.excluded.length > 0 && <details className="details"><summary>已排除 · {receipt.excludedCount} 项</summary><ul>{receipt.excluded.map(item => <li key={item.assetId}>{item.name || item.importName || item.assetId}：{item.message}</li>)}</ul></details>}
      <details className="details"><summary>导出清单位置</summary><code className="path-value">{receipt.manifestPath}</code><p className="muted">清单记录精确文件名、采用版本及未导出原因；图片文件夹仅包含 PNG。</p></details>
      <div className="form-actions"><Button onClick={() => setReceipt(null)}>关闭</Button></div>
    </Modal>}
  </>;
}
