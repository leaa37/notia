import { memo, Suspense, useCallback, lazy } from 'react'
import { isTextFileDocument, type OpenFileDocument } from '../../../types/views/fileDocument'
import type { MarkdownWikiLinkTarget } from '../../../types/views/markdownWikiLink'
import type { MarkdownDocumentUpdate, MarkdownSelectionContext } from '../../../types/views/markdownSelection'
import { shouldUseLargeMarkdownView } from '../../../engines/markdown/markdownEditorLimits'
import { ImageView } from './ImageView'
import { LargeMarkdownView } from './LargeMarkdownView'
import { TextView } from './TextView'
import type { LibraryContext } from '../../../services/contexts/libraryContexts'
import type { MarkdownPageLayout } from '../../../services/preferences/editorPreferences'

const MarkdownView = lazy(async () => {
  const module = await import('./MarkdownView')
  return { default: module.MarkdownView }
})
const MermaidView = lazy(async () => {
  const module = await import('./mermaid/MermaidView')
  return { default: module.MermaidView }
})

function FileViewFallback() {
  return (
    <div className="notia-main" role="status" aria-live="polite">
      <div className="notia-workspace-deferred-view">
        <div className="notia-workspace-deferred-card">
          <strong>Cargando editor...</strong>
        </div>
      </div>
    </div>
  )
}

interface FileViewHostProps {
  document: OpenFileDocument
  /** New source; the Markdown editor names its note (`documentPath`), the others write the active tab. */
  onTextSourceChange: (nextSource: string, documentPath?: string) => void
  /** A shared note changed and another editor saves it. */
  wikiLinkTargets: MarkdownWikiLinkTarget[]
  onOpenLinkedFile: (filePath: string) => void
  /** Creates a note next to the open one from a link property; resolves to an error message or `null`. */
  onCreateLinkedNote?: (title: string) => Promise<string | null>
  /** Page mode of Markdown notes; `null` draws them continuous. */
  pageLayout?: MarkdownPageLayout | null
  onSelectionChange: (selection: MarkdownSelectionContext | null) => void
  externalSourceUpdate: MarkdownDocumentUpdate | null
  theme: string
  markdownZoom: number
  onMarkdownZoomChange: (zoom: number) => void
  libraryId?: string
  contexts?: readonly LibraryContext[]
}

function ignoreSourceChange() {}

function FileViewHostComponent({
  document,
  onTextSourceChange,
  wikiLinkTargets,
  onOpenLinkedFile,
  onCreateLinkedNote,
  pageLayout = null,
  onSelectionChange,
  externalSourceUpdate,
  theme,
  markdownZoom,
  onMarkdownZoomChange,
  libraryId,
  contexts = [],
}: FileViewHostProps) {
  const handleMermaidSourcePersist = useCallback(async (nextSource: string) => {
    onTextSourceChange(nextSource)
  }, [onTextSourceChange])
  if (document.viewKind === 'image') {
    return <ImageView imageUrl={document.imageUrl} alt={document.name} />
  }

  if (document.viewKind === 'mermaid') {
    return (
      <Suspense fallback={<FileViewFallback />}>
        <MermaidView
          filePath={document.path}
          source={document.source}
          onSourcePersist={handleMermaidSourcePersist}
        />
      </Suspense>
    )
  }

  if (!isTextFileDocument(document)) {
    return null
  }

  if (document.viewKind === 'markdown') {
    // A read-only note never sends a save; the backend refuses it anyway.
    const onMarkdownSourceChange = document.readOnly ? ignoreSourceChange : onTextSourceChange
    if (shouldUseLargeMarkdownView(document.source)) {
      return <LargeMarkdownView source={document.source} onSourceChange={onMarkdownSourceChange} />
    }

    return (
      <Suspense fallback={<FileViewFallback />}>
        <MarkdownView
          key={document.path}
          source={document.source}
          documentPath={document.path}
          lockedContextTag={document.lockedContext}
          lockedContextSource={document.lockedContextSource}
          readOnly={document.readOnly}
          libraryId={libraryId}
          onSourceChange={onMarkdownSourceChange}
          wikiLinkTargets={wikiLinkTargets}
          onOpenLinkedFile={onOpenLinkedFile}
          onCreateLinkedNote={onCreateLinkedNote}
          pageLayout={pageLayout}
          onSelectionChange={onSelectionChange}
          externalSourceUpdate={externalSourceUpdate}
          theme={theme}
          zoom={markdownZoom}
          onZoomChange={onMarkdownZoomChange}
          contexts={contexts}
        />
      </Suspense>
    )
  }

  if (document.viewKind === 'text') {
    return <TextView source={document.source} onSourceChange={onTextSourceChange} />
  }

  return null
}

export const FileViewHost = memo(FileViewHostComponent)
FileViewHost.displayName = 'FileViewHost'
