import type { Page, Workspace } from './types.ts';

export const SERIES_WORKFLOW_PROFILE = 'desktop-series672-assets112/v1';
export type ProductionPage = 'foundation' | 'storyboard' | 'images' | 'videos';
export const PRODUCTION_PAGES: ProductionPage[] = ['foundation', 'storyboard', 'images', 'videos'];
const pageKey = (id: string) => `director.project-page.v1:${id}`;

export function rememberProjectPage(projectId: string, page: Page): void {
  if (!projectId || !PRODUCTION_PAGES.includes(page as ProductionPage)) return;
  try { globalThis.localStorage?.setItem(pageKey(projectId), page); } catch { /* Storage availability must not block project work. */ }
}

export function projectOpeningPage(workspace: Workspace, explicit?: Page): Page {
  if (explicit) return explicit;
  try {
    const saved = globalThis.localStorage?.getItem(pageKey(workspace.project.id));
    if (PRODUCTION_PAGES.includes(saved as ProductionPage)) return saved as ProductionPage;
  } catch { /* Preserve the established fallback when local storage is unavailable. */ }
  return workspace.lingmoDocuments?.length && !workspace.source ? 'videos' : 'foundation';
}
