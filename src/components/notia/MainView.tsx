import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { File, FileText, MessageSquare, PencilLine, Search } from 'lucide-react'
import { FileViewHost } from './views/FileViewHost' // memoized export
import { isTextFileDocument, type NotiaDocumentSaveStatus, type OpenFileDocument } from '../../types/views/fileDocument'
import type { MarkdownWikiLinkTarget } from '../../types/views/markdownWikiLink'
import { MAX_MARKDOWN_ZOOM, MIN_MARKDOWN_ZOOM } from './views/markdown/useMarkdownZoom'
import { MarkdownDocumentMenu } from './MarkdownDocumentMenu'
import { EditorSettingsModal, type EditorSettingsTab } from './editor-settings/EditorSettingsModal'
import { useEditorPreferences } from './hooks/useEditorPreferences'
import { useAppDispatch } from '../../store/hooks'
import { setDialogState } from '../../features/documents/documentsSlice'
import type { MarkdownExportFormat } from '../../modules/markdown-export/markdownExportEngine'
import type { MarkdownPageLayout } from '../../services/preferences/editorPreferences'
import { readNotePageMode, setNotePageMode } from '../../services/markdown/notePageModeRuntime'
import { leadingFrontmatterBlock } from '../../engines/markdown/frontmatterEngine'
import type { MarkdownDocumentUpdate, MarkdownSelectionContext } from '../../types/views/markdownSelection'
import type { LibraryContext } from '../../services/contexts/libraryContexts'
import type { NotiaLibrary } from '../../types/notia'
import { useNotiaAction } from '../../context/notiaActions/useNotiaAction'
import { backendSupports } from '../../services/transport'

const DEFAULT_MARKDOWN_ZOOM = 1

interface MainViewProps {
  activeDocument: OpenFileDocument | null
  saveStatus: NotiaDocumentSaveStatus
  onTextDocumentChange: (nextSource: string, documentPath?: string) => void
  markdownWikiLinkTargets: MarkdownWikiLinkTarget[]
  onOpenLinkedFile: (filePath: string) => void
  /** Creates a note next to the open one from a link property; resolves to an error message or `null`. */
  onCreateLinkedNote?: (title: string) => Promise<string | null>
  onSelectionChange: (selection: MarkdownSelectionContext | null) => void
  externalSourceUpdate: MarkdownDocumentUpdate | null
  theme: string
  contexts?: readonly LibraryContext[]
  activeLibrary?: NotiaLibrary | null
}

function getSaveStatusLabel(status: NotiaDocumentSaveStatus): string {
  if (status === 'saving') {
    return 'Guardando...'
  }

  if (status === 'error') {
    return 'Error al guardar'
  }

  return 'Guardado'
}

/** No note open: what to start with. The app window's actions, so only the app shows it. */
function MainViewEmpty({ canStart }: { canStart: boolean }) {
  const handleExplorerToolClick = useNotiaAction('explorerToolClick')
  const handleHeaderActionClick = useNotiaAction('headerActionClick')
  const handleRailActionClick = useNotiaAction('railActionClick')
  return (
    <main className="notia-main" data-notia-prevent-menu-close>
      <div className="notia-main-empty">
        <div className="notia-main-empty-mark" aria-hidden="true">
          <FileText size={26} strokeWidth={1.5} />
        </div>
        <div className="notia-main-empty-copy">
          <h2>No hay ninguna nota abierta</h2>
          <p>Elegí un archivo del explorador o empezá uno nuevo.</p>
        </div>
        <div className="notia-main-empty-actions">
          <button
            type="button"
            className="notia-main-empty-action"
            disabled={!canStart}
            onClick={() => handleExplorerToolClick('new-note')}
          >
            <span className="notia-main-empty-action-label">
              <PencilLine size={15} strokeWidth={1.75} aria-hidden="true" />
              Nueva nota
            </span>
            <kbd>Ctrl N</kbd>
          </button>
          <button
            type="button"
            className="notia-main-empty-action"
            disabled={!canStart}
            onClick={() => handleRailActionClick('chat')}
          >
            <span className="notia-main-empty-action-label">
              <MessageSquare size={15} strokeWidth={1.75} aria-hidden="true" />
              Nuevo chat
            </span>
          </button>
          <button
            type="button"
            className="notia-main-empty-action"
            disabled={!canStart}
            onClick={() => handleHeaderActionClick('search')}
          >
            <span className="notia-main-empty-action-label">
              <Search size={15} strokeWidth={1.75} aria-hidden="true" />
              Ir a archivo
            </span>
            <kbd>Ctrl O</kbd>
          </button>
        </div>
      </div>
    </main>
  )
}

function MainViewComponent({
  activeDocument,
  saveStatus,
  onTextDocumentChange,
  markdownWikiLinkTargets,
  onOpenLinkedFile,
  onCreateLinkedNote,
  onSelectionChange,
  externalSourceUpdate,
  theme,
  contexts = [],
  activeLibrary = null,
}: MainViewProps) {
  const dispatch = useAppDispatch()
  const [markdownZoom, setMarkdownZoom] = useState(DEFAULT_MARKDOWN_ZOOM)
  const [exportingFormat, setExportingFormat] = useState<MarkdownExportFormat | null>(null)
  /** Open tab of the editor settings, or `null` when they are closed. */
  const [settingsTab, setSettingsTab] = useState<EditorSettingsTab | null>(null)
  const { editorPage, editorPageSetup } = useEditorPreferences()
  const isMarkdownOpen = activeDocument?.viewKind === 'markdown'
  const markdownSource = activeDocument && isTextFileDocument(activeDocument) && isMarkdownOpen ? activeDocument.source : null
  // Only the frontmatter is read again while the body is typed.
  const markdownFrontmatter = useMemo(() => (markdownSource === null ? null : leadingFrontmatterBlock(markdownSource)), [markdownSource])
  /** Page mode belongs to the note (`pageMode` property); without it the note opens in the normal editor. */
  const notePageMode = useMemo(() => (markdownFrontmatter === null ? false : readNotePageMode(markdownFrontmatter)), [markdownFrontmatter])
  const latestDocumentRef = useRef({ path: activeDocument?.path ?? null, source: markdownSource })
  latestDocumentRef.current = { path: activeDocument?.path ?? null, source: markdownSource }

  useEffect(() => {
    setMarkdownZoom(DEFAULT_MARKDOWN_ZOOM)
  }, [activeDocument?.path])

  // Ctrl+, opens the editor settings, as the «⋯» menu shows.
  useEffect(() => {
    if (!isMarkdownOpen) return
    const openSettings = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key === ',') {
        event.preventDefault()
        setSettingsTab('page')
      }
    }
    window.addEventListener('keydown', openSettings)
    return () => window.removeEventListener('keydown', openSettings)
  }, [isMarkdownOpen])

  const pageLayout = useMemo<MarkdownPageLayout | null>(() => (
    editorPageSetup
      ? {
        paged: notePageMode === true,
        widthMm: notePageMode ? editorPageSetup.widthMm : editorPageSetup.continuousWidthMm,
        heightMm: editorPageSetup.heightMm,
        marginMm: editorPageSetup.marginMm,
        pageNumbers: editorPageSetup.pageNumbers,
      }
      : null
  ), [notePageMode, editorPageSetup])
  const formatLabel = editorPageSetup?.formats.find((format) => format.id === editorPage?.format)?.label ?? ''
  const orientationLabel = editorPage?.orientation === 'landscape' ? 'Horizontal' : 'Vertical'

  /** Writes or removes the note's `pageMode` property through Rust. */
  const togglePageMode = useCallback(async () => {
    const { path, source } = latestDocumentRef.current
    if (!path || source === null) return
    try {
      const nextSource = await setNotePageMode(source, !readNotePageMode(source))
      // Typing or another tab in the meantime wins; the toggle is dropped.
      const latest = latestDocumentRef.current
      if (latest.path !== path || latest.source !== source) return
      onTextDocumentChange(nextSource)
    } catch (error) {
      dispatch(setDialogState({
        type: 'info',
        title: 'No se pudo cambiar el modo página',
        message: error instanceof Error ? error.message : 'No se pudo cambiar el modo página de la nota.',
      }))
    }
  }, [dispatch, onTextDocumentChange])

  const handleExport = useCallback(async (format: MarkdownExportFormat) => {
    if (!activeDocument || !isTextFileDocument(activeDocument) || activeDocument.viewKind !== 'markdown') return
    setExportingFormat(format)
    try {
      const { exportMarkdownDocument } = await import('../../modules/markdown-export/markdownExportEngine')
      // Exports are rendered and written next to the source document by the
      // Rust backend on Windows and Android (SAF); the WebView never writes.
      await exportMarkdownDocument(format, {
        libraryId: activeLibrary?.id ?? null,
        sourceDocumentPath: activeDocument.path,
      })
    } catch (error) {
      dispatch(setDialogState({
        type: 'info',
        title: 'No se pudo exportar',
        message: error instanceof Error ? error.message : 'No se pudo exportar el documento.',
      }))
    } finally {
      setExportingFormat(null)
    }
  }, [activeDocument, activeLibrary, dispatch])

  if (!activeDocument) {
    return <MainViewEmpty canStart={Boolean(activeLibrary)} />
  }

  const extensionLabel = activeDocument.extension ? `.${activeDocument.extension}` : 'sin extension'
  const isTextDocument = isTextFileDocument(activeDocument)
  const isMarkdownDocument = activeDocument.viewKind === 'markdown'
  const markdownZoomPercent = Math.round(markdownZoom * 100)
  // The file is written next to the note: only the app can hand it over.
  const canExport = backendSupports('backend_export_markdown_document')

  return (
    <main className="notia-main" data-notia-prevent-menu-close>
      <header className="notia-main-header" data-notia-prevent-menu-close>
        <div className="notia-main-title-group" data-notia-prevent-menu-close>
          <h2>{activeDocument.name}</h2>
          <span>{extensionLabel}</span>
        </div>
        <div className="notia-main-header-actions">
          {isTextDocument ? (
            <span className={`notia-main-save-status notia-main-save-status--${saveStatus}`}>
              {getSaveStatusLabel(saveStatus)}
            </span>
          ) : null}
          {isMarkdownDocument && notePageMode && formatLabel ? (
            <button type="button" className="notia-page-chip" onClick={() => setSettingsTab('page')} title="Configurar la página">
              <File size={12} aria-hidden="true" />
              {formatLabel} · {orientationLabel}
            </button>
          ) : null}
          {isMarkdownDocument ? (
            <div className="notia-markdown-zoom-control" aria-label="Zoom del documento Markdown">
              <label htmlFor="notia-markdown-zoom" title="Zoom del documento">
                {markdownZoomPercent}%
              </label>
              <input
                id="notia-markdown-zoom"
                type="range"
                min={MIN_MARKDOWN_ZOOM * 100}
                max={MAX_MARKDOWN_ZOOM * 100}
                step="5"
                value={markdownZoomPercent}
                onChange={(event) => setMarkdownZoom(Number(event.target.value) / 100)}
                aria-valuetext={`${markdownZoomPercent}%`}
              />
              <button
                type="button"
                onClick={() => setMarkdownZoom(DEFAULT_MARKDOWN_ZOOM)}
                disabled={markdownZoom === DEFAULT_MARKDOWN_ZOOM}
              >
                Restablecer
              </button>
              <MarkdownDocumentMenu
                pageMode={notePageMode}
                pageSizeLabel={notePageMode && formatLabel ? formatLabel : 'Continuo'}
                canExportPdf={notePageMode}
                exportingFormat={exportingFormat}
                onTogglePageMode={() => void togglePageMode()}
                onOpenPageSettings={() => setSettingsTab('page')}
                onExport={canExport ? (format) => void handleExport(format) : undefined}
              />
            </div>
          ) : null}
        </div>
      </header>
      <section className="notia-main-content" data-notia-prevent-menu-close>
        <FileViewHost
          document={activeDocument}
          onTextSourceChange={onTextDocumentChange}
          wikiLinkTargets={markdownWikiLinkTargets}
          onOpenLinkedFile={onOpenLinkedFile}
          onCreateLinkedNote={onCreateLinkedNote}
          onSelectionChange={onSelectionChange}
          externalSourceUpdate={externalSourceUpdate}
          theme={theme}
          markdownZoom={markdownZoom}
          onMarkdownZoomChange={setMarkdownZoom}
          libraryId={activeLibrary?.id}
          contexts={contexts}
          pageLayout={pageLayout}
        />
      </section>
      <EditorSettingsModal
        open={settingsTab !== null}
        tab={settingsTab ?? 'page'}
        onTabChange={setSettingsTab}
        onClose={() => setSettingsTab(null)}
        pageMode={isMarkdownDocument ? notePageMode : null}
        onTogglePageMode={() => void togglePageMode()}
      />
    </main>
  )
}

export const MainView = memo(MainViewComponent)
MainView.displayName = 'MainView'
