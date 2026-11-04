import { backendFileUrl, callBackend } from '../../../services/transport'
import { memo, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from 'react'
import { Lock } from 'lucide-react'
import { useAppDispatch, useAppSelector } from '../../../store/hooks'
import { setRightChatPanelOpen } from '../../../features/ui/uiSlice'
import { requestChatPanel } from '../../../services/chat/chatComposerRequests'
import { selectAiSettings, selectInkMathPreferences, selectTheme } from '../../../features/preferences/preferencesSelectors'
import { Crepe } from '@milkdown/crepe'
import { editorViewCtx, parserCtx, serializerCtx } from '@milkdown/kit/core'
import { NodeSelection, TextSelection } from '@milkdown/kit/prose/state'
import type { Node as ProseNode } from '@milkdown/kit/prose/model'
import type { EditorView } from '@milkdown/kit/prose/view'
import { gfm } from '@milkdown/preset-gfm'
import { emoji } from '@milkdown/plugin-emoji'
import { cursor } from '@milkdown/plugin-cursor'
import { indent } from '@milkdown/plugin-indent'
import { trailing } from '@milkdown/plugin-trailing'
import { clipboard } from '@milkdown/plugin-clipboard'
import { codeBlockConfig } from '@milkdown/kit/component/code-block'
import { commandsCtx } from '@milkdown/kit/core'
import { blockConfig } from '@milkdown/kit/plugin/block'
import {
  codeBlockSchema,
  clearTextInCurrentBlockCommand,
  setBlockTypeCommand,
} from '@milkdown/kit/preset/commonmark'
import { tableCellSchema, tableHeaderSchema } from '@milkdown/preset-gfm'
import {
  leadingFrontmatterBlock,
  parseFrontmatterDocument,
  serializeFrontmatterDocument,
  type FrontmatterEntry,
} from '../../../engines/markdown/frontmatterEngine'
import {
  buildWikiLinkLookup,
  type MarkdownWikiLinkLookup,
} from '../../../engines/markdown/wikiLinkEngine'
import type { MarkdownWikiLinkTarget } from '../../../types/views/markdownWikiLink'
import type { MarkdownDocumentUpdate, MarkdownSelectionContext } from '../../../types/views/markdownSelection'
import { buildMarkdownSelectionContext } from '../../../engines/markdown/selectionEngine'
import { MarkdownPropertiesPanel } from './MarkdownPropertiesPanel'
import {
  WikiLinkSuggestionMenu,
  type WikiLinkSuggestionMenuState,
} from './markdown/WikiLinkSuggestionMenu'
import { configureWikiLinkSerializer, createWikiLinkPlugin, type WikiLinkMenuContext } from './markdown/wikiLinkPlugin'
import { sheetFitScale, useMarkdownZoom } from './markdown/useMarkdownZoom'
import { leaveStylusToSystem, marqueeTakesPress } from './markdown/stylusWriting'
import { createXGraphPlaceholder, isXGraphLanguage } from '../../../engines/markdown/xgraphEngine'
import { observeXGraphPreviews } from '../../../services/markdown/xgraphPreviewRuntime'
import './markdown/xgraph.css'
import { quickHash } from '../../../modules/mermaid/engines/mermaidEngine'
import { mountInlineMermaidPreview, unmountInlineMermaidPreview } from '../../../modules/mermaid/services/mermaidPreviewRuntime'
import { createInkMathApp, type InkMathHostBridge } from '../../../modules/inkmath/platform/inkmathPlatform'
import { InkMathModal } from '../../../modules/inkmath/view/InkMathModal'
import { setCompatibleIcon } from '../../../modules/inkmath/view/iconFallback'
import { recognizeInkMathWithAi } from '../../../services/ai/aiRuntime'
import '@milkdown/crepe/theme/common/style.css'
import '@milkdown/crepe/theme/nord.css'
import '../../../modules/inkmath/inkmath.css'
import type { LibraryContext } from '../../../services/contexts/libraryContexts'
import {
  extendTableCellSchemaWithBlocks,
  tableCellBlocksRemark,
} from '../../../engines/markdown/tableCellBlocks'
import { shouldShowMarkdownBlockHandle } from '../../../engines/markdown/markdownBlockHandleEngine'
import { markdownTableBlockView } from './markdown/markdownTableBlockView'
import { ChatAttachmentImages } from './ChatAttachmentImages'
import { suggestLinkTargets } from '../../../services/libraries/libraryLinkRuntime'
import { $prose, replaceAll } from '@milkdown/kit/utils'
import { toggleLinkCommand } from '@milkdown/kit/component/link-tooltip'
import { configureBlockAlignment, richTextPlugins } from './markdown/richTextMarks'
import {
  activeBlockPlugin,
  attachBlockDragGhost,
  BLOCK_GRIP_ICON,
  blockHandleReference,
  hideBlockHandleOnPointerLeave,
  isPageBreakRow,
  observeBlockHandleVisibility,
  setActiveBlock,
  trackPointerRow,
} from './markdown/blockHandle'
import { createFormatToolbarPlugin, type FormatToolbarState } from './markdown/formatToolbarPlugin'
import { createBlockGapPlugin, handleClickBelowContent } from './markdown/blockGapPlugin'
import { attachBlockMarquee, createBlockSelectionPlugin, isMarqueeTarget } from './markdown/blockMarquee'
import {
  clearFormatting,
  selectBlockText,
  setBlockAlignment,
  setBlockKind,
  setColorMark,
  toggleFormatMark,
} from './markdown/formatCommands'
import { MarkdownFormatToolbar, type MarkdownFormatToolbarActions } from './markdown/MarkdownFormatToolbar'
import { createPaginationPlugin, requestPagination, type FlowBreak, type PaginationGeometry } from './markdown/paginationPlugin'
import type { MarkdownPageLayout } from '../../../services/preferences/editorPreferences'
import './markdown/markdownEditor.css'
import { createGitbookPlugins, GITBOOK_PART_NODE_NAMES } from './markdown/gitbook/gitbookPlugins'
import { addGitbookMenu, insertGitbookInline } from './markdown/gitbook/gitbookMenu'
import { GitbookResolver } from './markdown/gitbook/gitbookResolver'
import { editDrawing } from './markdown/gitbook/gitbookDrawingEditor'
import { decorateCodeBlocks } from './markdown/codeBlockChrome'
import { exportNoteDiagram } from '../../../services/markdown/noteDiagramExport'
import { mountMarkdownPreview } from './markdown/gitbook/gitbookMarkdownPreview'
import { resolveGitbookBlocks } from '../../../services/markdown/gitbookBlocksRuntime'
import './markdown/gitbook/gitbook.css'
import './markdown/editorElements.css'
import './markdown/ink/ink.css'
import { InkLayer, type PenBarTool } from './markdown/ink/InkLayer'
import { MarkdownPenBar } from './markdown/ink/MarkdownPenBar'
import { useNoteInk } from './markdown/ink/useNoteInk'
import { insertInkBlock } from './markdown/ink/inkInsertion'
import { inkBottom } from './markdown/ink/inkPaths'
import { flowToPage, isPageStroke, strokeInFlow } from './markdown/ink/inkFlow'
import { useEditorPreferences } from '../hooks/useEditorPreferences'
import { WikiLinkPreviewCard } from './markdown/WikiLinkPreviewCard'
import { codeHighlight } from './markdown/codeHighlight'
import { createMarkdownChangeBuffer, createMarkdownChangePlugin } from './markdown/markdownChangeBuffer'
import { createSelectionNotifier, type SelectionNotifier } from './markdown/selectionNotifier'
import { registerPendingEditorChanges } from '../../../services/markdown/pendingEditorChanges'

const WIKI_LINK_MENU_WIDTH = 320
const WIKI_LINK_MENU_MARGIN = 12
const MATH_CODE_BLOCK_LANGUAGES = new Set(['latex', 'math', 'tex'])

const LIST_BULLET_ICON = '<svg viewBox="0 0 6 6"><circle cx="3" cy="3" r="2.35"/><rect x="0.6" y="0.6" width="4.8" height="4.8"/></svg>'
const TASK_CHECKED_ICON = '<svg viewBox="0 0 18 18"><rect class="notia-check-box" x="0" y="0" width="18" height="18" rx="5"/><path class="notia-check-mark" d="M4.9 9.6l2.9 2.7 5.3-6.1" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>'
const TASK_UNCHECKED_ICON = '<svg viewBox="0 0 18 18"><rect class="notia-check-box" x="0.75" y="0.75" width="16.5" height="16.5" rx="4.25" stroke-width="1.5"/></svg>'
const CODE_HIDE_ICON = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M2 2l12 12M6.5 6.6A2 2 0 009.4 9.5M4.3 4.4C2.9 5.4 2 6.8 1.5 8c1.2 2.8 3.8 4.5 6.5 4.5 1.3 0 2.5-.4 3.6-1M7 3.6c.3 0 .7-.1 1-.1 2.7 0 5.3 1.7 6.5 4.5-.3.7-.8 1.5-1.4 2.1"/></svg>'
const CODE_SHOW_ICON = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 6l3.5 3.5L11.5 6"/></svg>'
const OCR_BUTTON_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/></svg>'
const INLINE_LATEX_TOOLTIP_SELECTOR = '.milkdown-latex-inline-edit'
const INLINE_LATEX_BUTTON_SELECTOR = '[data-notia-inkmath-inline-button]'

interface MarkdownViewProps {
  source: string
  documentPath: string
  /** Context the note keeps because it lives in a Task Manager board or in a folder with a context. */
  lockedContextTag?: string
  lockedContextSource?: 'board' | 'folder'
  /** Why the note is read-only (a file only the AI writes): shown above it, nothing in it can be changed. */
  readOnly?: string
  libraryId?: string
  /**
   * The note's new Markdown, with the path of the note it belongs to: the
   * editor writes it once typing pauses, and on closing, after another tab
   * may already be the active one.
   */
  onSourceChange: (nextSource: string, documentPath: string) => void
  wikiLinkTargets: MarkdownWikiLinkTarget[]
  onOpenLinkedFile: (filePath: string) => void
  onSelectionChange: (selection: MarkdownSelectionContext | null) => void
  externalSourceUpdate: MarkdownDocumentUpdate | null
  theme?: string
  zoom: number
  onZoomChange: (zoom: number) => void
  contexts?: readonly LibraryContext[]
  /** Creates a note next to this one from a link property; resolves to an error message or `null`. */
  onCreateLinkedNote?: (title: string) => Promise<string | null>
  /** Page mode: the note is drawn on sheets of this size. */
  pageLayout?: MarkdownPageLayout | null
  /** What an empty line invites to write. */
  placeholder?: string
}

/** Drop line of a dragged block, as thin as the design's. */
const DROP_INDICATOR_WIDTH = 2
/**
 * Distance from the text to the handle: the painted block reaches 12px left
 * of the text (10px past the column), and the grip keeps 8px from it.
 */
const BLOCK_HANDLE_OFFSET = 20
/** CSS pixels per millimetre (96 dpi), so a sheet measures as the paper. */
const PX_PER_MM = 96 / 25.4
/** Space between two sheets, as in the canvas. */
const PAGE_GAP_PX = 32
/** Room the page number keeps above the bottom margin. */
const PAGE_NUMBER_BAND_PX = 18
/** Background around the sheets; narrower on phones. */
const DESK_PADDING_PX = 40
const NARROW_DESK_PADDING_PX = 16
const NARROW_EDITOR_PX = 600
/** Room below the lowest stroke of the continuous sheet; more while drawing, to keep writing. */
const INK_TAIL_PX = 120
const INK_DRAWING_TAIL_PX = 480
/** The sheet grows by a screen when less than this share of a screen is left below. */
const ENDLESS_SHEET_TRIGGER = 0.5
const NO_CONTEXTS: readonly LibraryContext[] = []
/** Elements whose chrome the editor adds: code blocks and the inline LaTeX editor. */
const CHROME_HOST_SELECTOR = `.milkdown-code-block, ${INLINE_LATEX_TOOLTIP_SELECTOR}`

/**
 * Whether a change of the editor's DOM may need chrome: it happens in a code
 * block or the inline LaTeX editor, or brings one. Typing in a paragraph
 * does not.
 */
function touchesEditorChrome(record: MutationRecord): boolean {
  const target = record.target instanceof Element ? record.target : record.target.parentElement
  if (target?.closest(CHROME_HOST_SELECTOR)) return true
  for (const node of Array.from(record.addedNodes)) {
    if (node instanceof Element && (node.matches(CHROME_HOST_SELECTOR) || node.querySelector(CHROME_HOST_SELECTOR))) return true
  }
  return false
}

interface PagePixels {
  width: number
  height: number
  margin: number
  pageNumbers: boolean
}

function toPagePixels(layout: MarkdownPageLayout): PagePixels {
  return {
    width: Math.round(layout.widthMm * PX_PER_MM),
    height: Math.round(layout.heightMm * PX_PER_MM),
    margin: Math.round(layout.marginMm * PX_PER_MM),
    pageNumbers: layout.pageNumbers,
  }
}

interface SheetFrameProps {
  pagesRef: RefObject<HTMLDivElement | null>
  /** Width of the sheet before scaling; `null` without a sheet. */
  sheetWidth: number | null
  scale: number
  children: ReactNode
}

/**
 * Holds the scaled sheet with its scaled size. It follows the height of the
 * sheet on its own, so a new line draws this frame again, not the editor.
 */
function SheetFrame({ pagesRef, sheetWidth, scale, children }: SheetFrameProps) {
  // Height of the sheet before scaling.
  const [pagesHeight, setPagesHeight] = useState(0)
  useEffect(() => {
    const pages = pagesRef.current
    if (!pages || typeof ResizeObserver !== 'function') return
    // A transform does not change the layout height the observer reports.
    const observer = new ResizeObserver(() => setPagesHeight(pages.offsetHeight))
    observer.observe(pages)
    return () => observer.disconnect()
  }, [pagesRef])
  const style = sheetWidth === null
    ? undefined
    : { width: sheetWidth * scale, height: pagesHeight * scale } as CSSProperties
  return <div className="notia-markdown-pages-frame" style={style}>{children}</div>
}

function clampWikiLinkMenuLeft(left: number): number {
  const maxLeft = Math.max(WIKI_LINK_MENU_MARGIN, window.innerWidth - WIKI_LINK_MENU_WIDTH - WIKI_LINK_MENU_MARGIN)
  return Math.min(Math.max(left, WIKI_LINK_MENU_MARGIN), maxLeft)
}

function areSuggestionListsEqual(
  left: MarkdownWikiLinkTarget[],
  right: MarkdownWikiLinkTarget[],
): boolean {
  if (left.length !== right.length) {
    return false
  }

  for (let index = 0; index < left.length; index += 1) {
    if (left[index]?.path !== right[index]?.path) {
      return false
    }
  }

  return true
}

function isSameMenuState(
  left: WikiLinkSuggestionMenuState | null,
  right: WikiLinkSuggestionMenuState | null,
): boolean {
  if (left === right) {
    return true
  }

  if (!left || !right) {
    return false
  }

  return (
    left.query === right.query &&
    left.replaceFrom === right.replaceFrom &&
    left.replaceTo === right.replaceTo &&
    left.anchorLeft === right.anchorLeft &&
    left.anchorTop === right.anchorTop &&
    left.selectedIndex === right.selectedIndex &&
    areSuggestionListsEqual(left.suggestions, right.suggestions)
  )
}

function buildWikiLinkMenuState(
  context: WikiLinkMenuContext | null,
  currentState: WikiLinkSuggestionMenuState | null,
  suggestions: MarkdownWikiLinkTarget[],
): WikiLinkSuggestionMenuState | null {
  if (!context || suggestions.length === 0) {
    return null
  }

  const previousSelectionPath =
    currentState && currentState.suggestions.length > 0
      ? currentState.suggestions[currentState.selectedIndex]?.path
      : undefined

  let selectedIndex = 0
  if (previousSelectionPath) {
    const index = suggestions.findIndex((target) => target.path === previousSelectionPath)
    if (index >= 0) {
      selectedIndex = index
    }
  }

  return {
    query: context.query,
    replaceFrom: context.replaceFrom,
    replaceTo: context.replaceTo,
    anchorLeft: clampWikiLinkMenuLeft(context.anchorLeft),
    anchorTop: Math.max(context.anchorTop, WIKI_LINK_MENU_MARGIN),
    suggestions,
    selectedIndex,
  }
}

function insertWikiLinkSuggestion(
  view: EditorView,
  menuState: WikiLinkSuggestionMenuState,
  target: MarkdownWikiLinkTarget,
): void {
  const replacement = `[[${target.wikiLink}]]`
  let transaction = view.state.tr.insertText(replacement, menuState.replaceFrom, menuState.replaceTo)
  const nextSelection = menuState.replaceFrom + replacement.length
  transaction = transaction.setSelection(TextSelection.create(transaction.doc, nextSelection))
  view.dispatch(transaction.scrollIntoView())
}

const MERMAID_KEYWORDS = [
  'erDiagram', 'flowchart', 'graph ', 'graph\t', 'graph\n',
  'sequenceDiagram', 'classDiagram', 'stateDiagram', 'stateDiagram-v2',
  'gantt', 'pie', 'gitGraph', 'mindmap', 'journey', 'requirementDiagram',
  'c4Context', 'c4Container', 'c4Component', 'c4Dynamic', 'c4Deployment',
  'timeline', 'sankey-beta', 'xychart-beta', 'block-beta', 'packet-beta',
  'kanban', 'architecture-beta',
]

function looksLikeMermaid(content: string): boolean {
  const trimmed = content.trim()
  if (!trimmed) return false
  const firstLine = trimmed.split('\n')[0]?.trim().toLowerCase() || ''
  return MERMAID_KEYWORDS.some((kw) => firstLine.startsWith(kw.toLowerCase()))
}

function renderMermaidPreview(
  _language: string,
  content: string,
  blockIndex: number,
  applyPreview: (html: string) => void,
): void {
  // Use deterministic ids so repeated renderPreview calls for the same block
  // reuse the existing host instead of creating orphan placeholders.
  const uniqueId = `${quickHash(content)}_${blockIndex}`
  const containerId = `notia-mmd-host-${uniqueId}`
  const storageKey = `mmd-inline-${uniqueId}`

  // If the host is already in the DOM, Milkdown already rendered this block.
  // Skip re-injection to avoid flashing and orphan React roots.
  if (document.getElementById(containerId)) {
    return
  }

  // Render a placeholder with visible text and a minimum size so DOMPurify
  // (used by Milkdown) does not purge the container and the block never
  // collapses to 0 height while the diagram loads.
  const placeholder = `<div id="${containerId}" class="notia-mermaid-inline-host" data-code="${encodeURIComponent(content)}" data-storage-key="${storageKey}" style="min-height:160px;width:100%;" data-render-id="${uniqueId}"><div style="padding:12px;color:var(--color-icon-muted);font-size:13px;">Renderizando diagrama...</div></div>`
  applyPreview(placeholder)

  requestAnimationFrame(() => {
    const host = document.getElementById(containerId)
    if (!host || !host.isConnected) {
      // Milkdown may have replaced the placeholder before the frame ran.
      // This is expected during fast re-renders; do not spam warnings.
      return
    }
    mountInlineMermaidPreview(host, content, storageKey)
  })
}

/**
 * The text is only edited with the selector: with a drawing tool no press
 * puts the caret in the note (a palm, the desk around the sheet, the gap
 * between pages) and the stylus is not taken for the system's handwriting.
 */
function followPenTool(crepe: Crepe, tool: PenBarTool, readOnly: boolean) {
  const drawing = tool !== 'selector'
  crepe.setReadonly(readOnly || drawing)
  const dom = crepe.editor.action((ctx) => ctx.get(editorViewCtx)).dom
  if (drawing && document.activeElement instanceof HTMLElement && dom.contains(document.activeElement)) document.activeElement.blur()
}

function createMarkdownInkMathApp(getDocumentPath: () => string) {
  const unavailable = async (): Promise<never> => {
    throw new Error('Esta operación no está disponible desde el editor Markdown.')
  }
  const bridge: InkMathHostBridge = {
    rootPath: '',
    getActiveFilePath: getDocumentPath,
    listFilePaths: () => [],
    readFile: unavailable,
    writeFile: unavailable,
    createFile: unavailable,
    createFolder: unavailable,
    deletePath: unavailable,
    existsPath: async () => false,
    writeBinaryFile: unavailable,
    onOpenFile: unavailable,
  }
  return createInkMathApp(bridge)
}

function findCodeBlockPosition(editorView: EditorView, codeBlockElement: HTMLElement): number | null {
  let domPosition: number
  try {
    domPosition = editorView.posAtDOM(codeBlockElement, 0)
  } catch {
    return null
  }

  for (const candidate of [domPosition, domPosition - 1]) {
    if (candidate >= 0 && editorView.state.doc.nodeAt(candidate)?.type.name === 'code_block') {
      return candidate
    }
  }

  const resolvedPosition = editorView.state.doc.resolve(domPosition)
  for (let depth = resolvedPosition.depth; depth > 0; depth -= 1) {
    if (resolvedPosition.node(depth).type.name === 'code_block') {
      return resolvedPosition.before(depth)
    }
  }

  return null
}

function readInlineLatexTarget(
  editorView: EditorView,
  tooltipContainer: HTMLElement,
): { position: number; latex: string; editorElement: HTMLElement | null } | null {
  const selection = editorView.state.selection
  if (!(selection instanceof NodeSelection) || selection.node.type.name !== 'math_inline') {
    return null
  }

  const editorElement = tooltipContainer.querySelector<HTMLElement>('.ProseMirror')
  const domLatex = editorElement?.textContent
  const latex = domLatex && domLatex.length > 0
    ? domLatex
    : String(selection.node.attrs.value ?? '')

  return {
    position: selection.from,
    latex,
    editorElement,
  }
}

function replaceInlineLatexEditorText(editorElement: HTMLElement, latex: string): void {
  editorElement.focus()
  const selection = window.getSelection()
  if (selection) {
    const range = document.createRange()
    range.selectNodeContents(editorElement)
    selection.removeAllRanges()
    selection.addRange(range)
  }

  let usedNativeEditing = false
  try {
    usedNativeEditing = document.execCommand('insertText', false, latex)
  } catch {
    usedNativeEditing = false
  }

  if (usedNativeEditing && editorElement.textContent === latex) {
    return
  }

  const paragraph = editorElement.querySelector<HTMLElement>('p') ?? editorElement
  paragraph.textContent = latex
  editorElement.dispatchEvent(new Event('input', { bubbles: true }))
}

function MarkdownViewInner({
  source,
  documentPath,
  lockedContextTag,
  lockedContextSource,
  readOnly,
  libraryId,
  onSourceChange,
  wikiLinkTargets,
  onOpenLinkedFile,
  onSelectionChange,
  externalSourceUpdate,
  zoom,
  onZoomChange,
  contexts = NO_CONTEXTS,
  onCreateLinkedNote,
  pageLayout = null,
  placeholder = 'Escribí acá…',
}: MarkdownViewProps) {
  const parsedDocument = useMemo(() => parseFrontmatterDocument(source), [source])
  // The properties panel reads only the frontmatter: typing in the body keeps
  // the same block, so the panel keeps the same entries and is not drawn again.
  const frontmatterBlock = useMemo(() => leadingFrontmatterBlock(source), [source])
  const frontmatterEntries = useMemo(() => parseFrontmatterDocument(frontmatterBlock).frontmatter, [frontmatterBlock])
  const wikiLinkLookup = useMemo(() => buildWikiLinkLookup(wikiLinkTargets), [wikiLinkTargets])

  const [wikiLinkMenuState, setWikiLinkMenuState] = useState<WikiLinkSuggestionMenuState | null>(null)
  const [isEditorReady, setIsEditorReady] = useState(false)
  const [formatToolbarState, setFormatToolbarState] = useState<FormatToolbarState | null>(null)

  const toolbarRef = useRef<HTMLDivElement | null>(null)
  const pagesRef = useRef<HTMLDivElement | null>(null)
  const [pageCount, setPageCount] = useState(1)
  // The sheet the note is written on: A3 pages, or one continuous sheet as
  // wide as an A3. Only pages are paginated.
  const sheetPixels = useMemo(() => (pageLayout ? toPagePixels(pageLayout) : null), [pageLayout])
  const pagePixels = pageLayout?.paged ? sheetPixels : null
  const [hostWidth, setHostWidth] = useState(0)
  const deskPadding = hostWidth > 0 && hostWidth < NARROW_EDITOR_PX ? NARROW_DESK_PADDING_PX : DESK_PADDING_PX
  // A sheet wider than the editor (a phone, a narrow window) is scaled down
  // to fit at 100 %; the person's zoom applies on top.
  const pageFit = sheetPixels ? sheetFitScale(hostWidth, deskPadding, sheetPixels.width) : 1
  // The sheet is drawn at `zoom × pageFit` with a transform, never with CSS
  // `zoom`: under `zoom` Chromium misplaced the caret and the selection, and
  // the Android keyboard (stylus writing too) wrote somewhere else, worse the
  // more it was zoomed. A transform keeps them on the text.
  const sheetScale = sheetPixels ? zoom * pageFit : 1
  const [hostHeight, setHostHeight] = useState(0)
  // Handwriting over the sheet: the tool of the pen bar and the note's strokes.
  const [penTool, setPenTool] = useState<PenBarTool>('selector')
  // The rectangle selection of blocks only works with the selector.
  const penToolRef = useRef<PenBarTool>('selector')
  useEffect(() => {
    penToolRef.current = penTool
  }, [penTool])
  const { pen, updatePen } = useEditorPreferences()
  const ink = useNoteInk(libraryId, documentPath)
  // Where the page breaks fall in the note's flow, once the pages are laid out.
  const [flowBreaks, setFlowBreaks] = useState<FlowBreak[]>([])
  const [pagesLaidOut, setPagesLaidOut] = useState(false)
  const inkSheet = useMemo(() => (sheetPixels
    ? {
      width: sheetPixels.width,
      paged: pagePixels !== null,
      pageHeight: sheetPixels.height,
      pageGap: PAGE_GAP_PX,
      margin: sheetPixels.margin,
      numberBand: sheetPixels.pageNumbers ? PAGE_NUMBER_BAND_PX : 0,
    }
    : null), [pagePixels, sheetPixels])
  // Strokes drawn on a page before strokes were kept in the flow move into
  // it the first time the pages are laid out; then both modes show them.
  const movedStrokesRef = useRef(new Set<string>())
  const { strokes: inkStrokes, replace: replaceInkStrokes } = ink
  // Read over every point: only again when the strokes change, not per keystroke.
  const lowestInk = useMemo(() => inkBottom(inkStrokes), [inkStrokes])
  useEffect(() => {
    if (!pagePixels || !pagesLaidOut) return
    const onPages = inkStrokes.filter((stroke) => isPageStroke(stroke) && !movedStrokesRef.current.has(stroke.id))
    if (onPages.length === 0) return
    onPages.forEach((stroke) => movedStrokesRef.current.add(stroke.id))
    replaceInkStrokes(onPages.map((stroke) => strokeInFlow(stroke, flowBreaks, pagePixels.height + PAGE_GAP_PX)))
  }, [flowBreaks, inkStrokes, pagePixels, pagesLaidOut, replaceInkStrokes])
  const paginationGeometryRef = useRef<PaginationGeometry | null>(null)
  const pointerRowRef = useRef<number | null>(null)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const viewportRef = useRef<HTMLDivElement | null>(null)
  const zoomContentRef = useRef<HTMLDivElement | null>(null)
  const crepeRef = useRef<Crepe | null>(null)
  const isReadyRef = useRef(false)
  const isApplyingExternalUpdateRef = useRef(false)
  const initialBodyRef = useRef(parsedDocument.body)
  // Read once, when the editor is built.
  const placeholderRef = useRef(placeholder)
  const latestComposedSourceRef = useRef(source)
  const latestBodyRef = useRef(parsedDocument.body)
  const frontmatterRef = useRef(parsedDocument.frontmatter)
  const hasFrontmatterRef = useRef(parsedDocument.hasFrontmatter)
  const documentPathRef = useRef(documentPath)
  const onSourceChangeRef = useRef(onSourceChange)
  const readOnlyRef = useRef(Boolean(readOnly))
  const wikiLinkSuggestionRequestRef = useRef(0)
  const libraryIdRef = useRef(libraryId)
  const wikiLinkLookupRef = useRef<MarkdownWikiLinkLookup>(wikiLinkLookup)
  const wikiLinkMenuStateRef = useRef<WikiLinkSuggestionMenuState | null>(null)
  const isWikiLinkMenuOpenRef = useRef(false)
  const onOpenLinkedFileRef = useRef(onOpenLinkedFile)
  const onSelectionChangeRef = useRef(onSelectionChange)
  const selectionNotifierRef = useRef<SelectionNotifier | null>(null)
  // Writes the Markdown the editor still holds (typing waits for a pause).
  const flushEditorChangesRef = useRef<() => void>(() => {})
  const selectionCleanupRef = useRef<(() => void) | null>(null)
  const mermaidPreviewBlockIndexRef = useRef(0)
  const gitbookResolverRef = useRef<GitbookResolver | null>(null)

  useMarkdownZoom(viewportRef, zoomContentRef, zoom, onZoomChange, !sheetPixels)

  // The sheet has no end: scrolling near its bottom adds a screen of blank
  // sheet (more pages in page mode). Only for this visit of the note; what
  // is written or drawn there keeps its room.
  const [scrolledSheetLength, setScrolledSheetLength] = useState(0)
  useEffect(() => {
    setScrolledSheetLength(0)
  }, [documentPath])
  const hasSheet = sheetPixels !== null
  useEffect(() => {
    const host = viewportRef.current
    const pages = pagesRef.current
    if (!hasSheet || !host || !pages) return
    const grow = () => {
      const left = host.scrollHeight - host.scrollTop - host.clientHeight
      if (left > host.clientHeight * ENDLESS_SHEET_TRIGGER) return
      // `offsetHeight` is the sheet before the zoom's transform.
      const length = Math.ceil(pages.offsetHeight + host.clientHeight / sheetScale)
      setScrolledSheetLength((current) => Math.max(current, length))
    }
    grow()
    host.addEventListener('scroll', grow, { passive: true })
    return () => host.removeEventListener('scroll', grow)
  }, [documentPath, hasSheet, sheetScale])

  useEffect(() => {
    const host = viewportRef.current
    if (!host || typeof ResizeObserver !== 'function') return
    const observer = new ResizeObserver(([entry]) => {
      setHostWidth(entry?.contentRect.width ?? 0)
      setHostHeight(entry?.contentRect.height ?? 0)
    })
    observer.observe(host)
    return () => observer.disconnect()
  }, [])

  // A new page setup, zoom or fit lays the page breaks again.
  useEffect(() => {
    paginationGeometryRef.current = pagePixels
      ? {
        pageHeight: pagePixels.height,
        pageGap: PAGE_GAP_PX,
        margin: pagePixels.margin,
        numberBand: pagePixels.pageNumbers ? PAGE_NUMBER_BAND_PX : 0,
        zoom: sheetScale,
      }
      : null
    const crepe = crepeRef.current
    if (!crepe || !isEditorReady) return
    requestPagination(crepe.editor.action((ctx) => ctx.get(editorViewCtx)))
  }, [isEditorReady, pagePixels, sheetScale])

  useEffect(() => {
    const crepe = crepeRef.current
    readOnlyRef.current = Boolean(readOnly)
    if (crepe && isEditorReady) followPenTool(crepe, penTool, Boolean(readOnly))
  }, [isEditorReady, penTool, readOnly])

  useEffect(() => {
    if (source === latestComposedSourceRef.current) {
      return
    }

    latestComposedSourceRef.current = source
    latestBodyRef.current = parsedDocument.body
    frontmatterRef.current = parsedDocument.frontmatter
    hasFrontmatterRef.current = parsedDocument.hasFrontmatter
    // If Crepe has not been created yet, make sure it will use the latest body
    // as its initial content. This handles the case where the first render has
    // an empty/stale source and the real content arrives before create().
    initialBodyRef.current = parsedDocument.body
  }, [parsedDocument, source])

  const appTheme = useAppSelector(selectTheme)
  const dispatch = useAppDispatch()
  // A prompt block's «Ejecutar» sends its text to the side chat, in a new chat.
  const runPromptRef = useRef((text: string) => {
    dispatch(setRightChatPanelOpen(true))
    requestChatPanel({ kind: 'send', text, agentFileName: null })
  })
  const inkMathPreferences = useAppSelector(selectInkMathPreferences)
  const aiPreferences = useAppSelector(selectAiSettings)
  const inkMathPreferencesRef = useRef(inkMathPreferences)
  const aiPreferencesRef = useRef(aiPreferences)
  const themeRef = useRef(appTheme)
  useEffect(() => {
    themeRef.current = appTheme
  }, [appTheme])

  useEffect(() => {
    inkMathPreferencesRef.current = inkMathPreferences
  }, [inkMathPreferences])

  useEffect(() => {
    aiPreferencesRef.current = aiPreferences
  }, [aiPreferences])

  // The lasso's AI button: Rust reads the strokes and returns the block; it
  // goes into the note at their height and then the strokes go (one undo
  // step of the pen bar; the editor's undo takes the block out).
  const { convert: convertInk, remove: removeInk } = ink
  const handleInkConvert = useCallback(async (ids: string[], top: () => number | null) => {
    const conversion = await convertInk(ids, aiPreferencesRef.current)
    const crepe = crepeRef.current
    if (!conversion || !crepe || !isReadyRef.current) return false
    const inserted = crepe.editor.action((ctx) => {
      try {
        return insertInkBlock(ctx.get(editorViewCtx), ctx.get(parserCtx)(conversion.markdown), top() ?? Number.POSITIVE_INFINITY)
      } catch {
        return false
      }
    })
    if (inserted) removeInk(conversion.ids)
    return inserted
  }, [convertInk, removeInk])

  useEffect(() => {
    onSourceChangeRef.current = onSourceChange
  }, [onSourceChange])

  useEffect(() => {
    libraryIdRef.current = libraryId
    wikiLinkLookupRef.current = wikiLinkLookup
  }, [libraryId, wikiLinkLookup])

  useEffect(() => {
    documentPathRef.current = documentPath
  }, [documentPath])

  // The GitBook blocks resolve against the library and path of the note.
  useEffect(() => {
    gitbookResolverRef.current?.invalidate()
  }, [libraryId, documentPath])

  // Expressions and conditions read the note's `vars:` property.
  useEffect(() => {
    gitbookResolverRef.current?.noteSourceChanged(source)
  }, [source])

  useEffect(() => {
    onOpenLinkedFileRef.current = onOpenLinkedFile
  }, [onOpenLinkedFile])

  useEffect(() => {
    onSelectionChangeRef.current = onSelectionChange
  }, [onSelectionChange])

  useEffect(() => {
    wikiLinkMenuStateRef.current = wikiLinkMenuState
    isWikiLinkMenuOpenRef.current = Boolean(wikiLinkMenuState)
  }, [wikiLinkMenuState])

  useEffect(() => {
    if (!rootRef.current) {
      return
    }

    const crepe = new Crepe({
      root: rootRef.current,
      defaultValue: initialBodyRef.current,
      features: {
        [Crepe.Feature.Toolbar]: false,
        [Crepe.Feature.BlockEdit]: true,
      },
      featureConfigs: {
        // The native caret: the virtual one measures in screen pixels inside
        // the scaled sheet, so it drifted right and down with the zoom.
        [Crepe.Feature.Cursor]: { width: DROP_INDICATOR_WIDTH, virtual: false },
        // The bar of the code blocks speaks Spanish, as in the canvas.
        [Crepe.Feature.CodeMirror]: {
          copyText: 'Copiar',
          searchPlaceholder: 'Buscar lenguaje',
          noResultText: 'Sin resultados',
          previewLabel: 'Vista previa',
          previewToggleText: (previewOnly) => (previewOnly ? 'Mostrar' : 'Ocultar código'),
          previewToggleIcon: (previewOnly) => (previewOnly ? CODE_SHOW_ICON : CODE_HIDE_ICON),
          theme: codeHighlight,
        },
        [Crepe.Feature.Placeholder]: { text: placeholderRef.current },
        // Teal dots, and the task boxes of the canvas.
        [Crepe.Feature.ListItem]: {
          bulletIcon: LIST_BULLET_ICON,
          checkBoxCheckedIcon: TASK_CHECKED_ICON,
          checkBoxUncheckedIcon: TASK_UNCHECKED_ICON,
        },
        [Crepe.Feature.BlockEdit]: {
          handleDragIcon: BLOCK_GRIP_ICON,
          blockHandle: {
            getOffset: () => BLOCK_HANDLE_OFFSET,
            // Centred on the first line, as in the canvas, whatever the block's padding.
            getPlacement: () => 'left',
            // The handle's block is also the one painted as active.
            getPosition: ({ ctx, active }) => {
              const view = ctx.get(editorViewCtx)
              setActiveBlock(view, active.$pos.pos)
              return blockHandleReference(view, active.$pos.pos, active.el)
            },
          },
          buildMenu: (builder) => {
            const advancedGroup = builder.getGroup('advanced')
            advancedGroup.addItem('xgraph', {
              label: 'XGraph',
              icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 3v18h18M4 16l5-8 5 5 6-9"/></svg>',
              onRun: (ctx) => {
                const commands = ctx.get(commandsCtx)
                commands.call(clearTextInCurrentBlockCommand.key)
                commands.call(setBlockTypeCommand.key, {
                  nodeType: codeBlockSchema.type(ctx),
                  attrs: { language: 'xgraph' },
                })
              },
            })
            addGitbookMenu(builder)
            advancedGroup.addItem('mermaid', {
              label: 'Mermaid',
              icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>',
              onRun: (ctx) => {
                const commands = ctx.get(commandsCtx)
                const codeBlock = codeBlockSchema.type(ctx)
                commands.call(clearTextInCurrentBlockCommand.key)
                commands.call(setBlockTypeCommand.key, {
                  nodeType: codeBlock,
                  attrs: { language: 'mermaid' },
                })
              },
            })
          },
        },
      },
    })

    let isMounted = true
    const cleanupXGraphPreviews = observeXGraphPreviews(rootRef.current)
    const gitbookResolver = new GitbookResolver((request) => {
      const currentLibraryId = libraryIdRef.current
      if (!currentLibraryId) return Promise.resolve(null)
      return resolveGitbookBlocks(currentLibraryId, documentPathRef.current, latestComposedSourceRef.current, request)
    }, latestComposedSourceRef.current)
    gitbookResolverRef.current = gitbookResolver
    const inlineHosts = new Set<HTMLElement>()
    const inkMathApp = createMarkdownInkMathApp(() => documentPathRef.current)

    const addMathOcrButtons = () => {
      const editorView = crepe.editor.action((ctx) => ctx.get(editorViewCtx))
      rootRef.current?.querySelectorAll<HTMLElement>('.milkdown-code-block').forEach((codeBlockElement) => {
        const toolsGroup = codeBlockElement.querySelector<HTMLElement>('.tools-button-group')
        if (!toolsGroup) {
          return
        }

        const language = codeBlockElement
          .querySelector<HTMLElement>('.language-button')
          ?.textContent?.toLowerCase().trim() ?? ''
        const existingButton = toolsGroup.querySelector('.notia-math-ocr-button')
        if (!MATH_CODE_BLOCK_LANGUAGES.has(language)) {
          existingButton?.remove()
          return
        }
        if (existingButton) {
          return
        }

        const button = document.createElement('button')
        button.type = 'button'
        button.className = 'notia-math-ocr-button'
        button.setAttribute('aria-label', 'Abrir OCR matemático')
        button.title = 'Abrir OCR matemático'
        button.innerHTML = `${OCR_BUTTON_ICON}<span>Escribir a mano</span>`
        button.addEventListener('click', () => {
          const currentPosition = findCodeBlockPosition(editorView, codeBlockElement)
          if (currentPosition === null) {
            return
          }
          const currentNode = editorView.state.doc.nodeAt(currentPosition)
          if (!currentNode || currentNode.type.name !== 'code_block') {
            return
          }
          const preferences = inkMathPreferencesRef.current
          new InkMathModal(inkMathApp, {
            initialOcrDebounceMs: preferences.debounceMs,
            recognizeLatex: (imageBase64, abortSignal) => recognizeInkMathWithAi(aiPreferencesRef.current, {
              name: 'inkmath-formula.png',
              mimeType: 'image/png',
              base64: imageBase64.replace(/^data:image\/png;base64,/, '').trim(),
            }, abortSignal),
            onAccept: (latex) => {
              const acceptedBlockPosition = findCodeBlockPosition(editorView, codeBlockElement)
              if (acceptedBlockPosition === null) {
                return
              }
              const acceptedBlockNode = editorView.state.doc.nodeAt(acceptedBlockPosition)
              if (!acceptedBlockNode || acceptedBlockNode.type.name !== 'code_block') {
                return
              }
              const transaction = editorView.state.tr.replaceWith(
                acceptedBlockPosition + 1,
                acceptedBlockPosition + acceptedBlockNode.nodeSize - 1,
                editorView.state.schema.text(latex),
              )
              editorView.dispatch(transaction.scrollIntoView())
              editorView.focus()
            },
          }).open()
        })
        toolsGroup.prepend(button)
      })
    }

    const addInlineLatexInkMathButtons = () => {
      rootRef.current?.querySelectorAll<HTMLElement>(`${INLINE_LATEX_TOOLTIP_SELECTOR} .container`).forEach((tooltipContainer) => {
        if (tooltipContainer.querySelector(INLINE_LATEX_BUTTON_SELECTOR)) {
          return
        }

        const button = document.createElement('button')
        button.type = 'button'
        button.className = 'notia-inkmath-inline-button'
        button.setAttribute('data-notia-inkmath-inline-button', 'true')
        button.setAttribute('aria-label', 'Abrir InkMath para editar LaTeX')
        button.title = 'Abrir InkMath para editar LaTeX'
        setCompatibleIcon(button, 'sigma', 'Σ')
        button.addEventListener('pointerdown', (event) => {
          event.preventDefault()
          event.stopPropagation()
        })
        button.addEventListener('click', (event) => {
          event.preventDefault()
          event.stopPropagation()

          const editorView = crepe.editor.action((ctx) => ctx.get(editorViewCtx))
          const target = readInlineLatexTarget(editorView, tooltipContainer)
          if (!target) {
            return
          }

          new InkMathModal(inkMathApp, {
            initialLatex: target.latex,
            initialOcrDebounceMs: inkMathPreferencesRef.current.debounceMs,
            recognizeLatex: (imageBase64, abortSignal) => recognizeInkMathWithAi(aiPreferencesRef.current, {
              name: 'inkmath-formula.png',
              mimeType: 'image/png',
              base64: imageBase64.replace(/^data:image\/png;base64,/, '').trim(),
            }, abortSignal),
            onAccept: (latex) => {
              if (!isMounted) {
                return
              }

              const nextLatex = latex.trim()
              const currentNode = editorView.state.doc.nodeAt(target.position)
              if (!nextLatex || !currentNode || currentNode.type.name !== 'math_inline') {
                return
              }

              if (target.editorElement?.isConnected) {
                replaceInlineLatexEditorText(target.editorElement, nextLatex)
              }

              editorView.dispatch(
                editorView.state.tr
                  .setNodeAttribute(target.position, 'value', nextLatex)
                  .scrollIntoView(),
              )

              // Milkdown may recreate the nested editor after the outer node
              // changes. Sync it once more so its confirm button cannot write
              // the previous value back into the math node.
              window.requestAnimationFrame(() => {
                const currentEditor = rootRef.current?.querySelector<HTMLElement>(
                  `${INLINE_LATEX_TOOLTIP_SELECTOR}[data-show="true"] .ProseMirror`,
                )
                if (currentEditor) {
                  replaceInlineLatexEditorText(currentEditor, nextLatex)
                }
                editorView.focus()
              })
            },
          }).open()
        })
        tooltipContainer.append(button)
      })
    }

    const codeBlockChrome = {
      isMathLanguage: (language: string) => MATH_CODE_BLOCK_LANGUAGES.has(language),
      exportDiagram: (format: 'svg' | 'png', data: string) => {
        const currentLibraryId = libraryIdRef.current
        if (!currentLibraryId) return Promise.reject(new Error('No hay una biblioteca abierta.'))
        return exportNoteDiagram(currentLibraryId, documentPathRef.current, format, data)
      },
    }
    // Chrome is added once per frame, and only after changes that may need it.
    let chromeFrame: number | null = null
    const addEditorChrome = () => {
      chromeFrame = null
      if (!isMounted) return
      addMathOcrButtons()
      addInlineLatexInkMathButtons()
      if (rootRef.current) decorateCodeBlocks(rootRef.current, codeBlockChrome)
    }
    const codeBlockObserver = new MutationObserver((records) => {
      if (chromeFrame !== null || !records.some(touchesEditorChrome)) return
      chromeFrame = window.requestAnimationFrame(addEditorChrome)
    })
    codeBlockObserver.observe(rootRef.current, { childList: true, characterData: true, subtree: true })

    const cleanupInlinePreviews = () => {
      rootRef.current?.querySelectorAll('.notia-mermaid-inline-host').forEach((node) => {
        const host = node as HTMLElement
        inlineHosts.add(host)
      })
      inlineHosts.forEach((host) => {
        unmountInlineMermaidPreview(host)
      })
      inlineHosts.clear()
    }

    const clearDocumentRefs = () => {
      latestComposedSourceRef.current = ''
      latestBodyRef.current = ''
      frontmatterRef.current = []
      hasFrontmatterRef.current = false
      initialBodyRef.current = ''
    }

    crepe.editor.config((ctx) => {
      ctx.update(codeBlockConfig.key, (prev) => ({
        ...prev,
        renderPreview: (language, content, applyPreview) => {
          const lowerLang = language.toLowerCase().trim()
          if (isXGraphLanguage(lowerLang)) return createXGraphPlaceholder(content)
          // Activar preview Mermaid en tres casos:
          // 1. Lenguaje explícitamente "mermaid"
          // 2. Lenguaje vacío pero contenido parece diagrama Mermaid
          // 3. Lenguaje "text" pero contenido parece diagrama Mermaid
          // Un bloque de texto plano queda sin vista previa, como en el canvas.
          const isMermaidLang = lowerLang === 'mermaid'
          const isEmptyLang = lowerLang === '' && looksLikeMermaid(content)
          const isTextButMermaid = lowerLang === 'text' && looksLikeMermaid(content)
          if (isMermaidLang || isEmptyLang || isTextButMermaid) {
            const blockIndex = mermaidPreviewBlockIndexRef.current
            mermaidPreviewBlockIndexRef.current += 1
            renderMermaidPreview(lowerLang, content, blockIndex, applyPreview)
            return undefined
          }
          return prev.renderPreview(language, content, applyPreview)
        },
      }))
      ctx.update(blockConfig.key, (prev) => ({
        ...prev,
        filterNodes: (pos, node) => {
          // No block between two sheets, as between top-level blocks.
          const pointerRow = pointerRowRef.current
          if (pointerRow !== null && isPageBreakRow(ctx.get(editorViewCtx), pointerRow)) {
            return false
          }
          let hasExcludedAncestor = false
          let isInsideTable = false
          for (let depth = pos.depth; depth > 0; depth -= 1) {
            const ancestor = pos.node(depth)
            if (ancestor.type.name === 'table') {
              isInsideTable = true
            }
            if (ancestor.type.name === 'blockquote' || ancestor.type.name === 'math_inline') {
              hasExcludedAncestor = true
            }
          }

          // Tabs, steps, columns and updates move with their block.
          if (GITBOOK_PART_NODE_NAMES.has(node.type.name)) {
            return false
          }

          return shouldShowMarkdownBlockHandle(node.type.name, hasExcludedAncestor, isInsideTable)
        },
      }))
      ctx.update(tableCellSchema.key, (prev) => () => extendTableCellSchemaWithBlocks(prev(ctx)))
      ctx.update(tableHeaderSchema.key, (prev) => () => extendTableCellSchemaWithBlocks(prev(ctx)))
      configureBlockAlignment(ctx)
      configureWikiLinkSerializer(ctx)
    })

    crepe.editor.use(richTextPlugins)
    crepe.editor.use(createGitbookPlugins({
      resolver: gitbookResolver,
      openLibraryPath: (path) => onOpenLinkedFileRef.current(path),
      fileUrl: backendFileUrl,
      editDrawing: (drawing) => editDrawing(drawing, rootRef.current),
      renderMarkdown: mountMarkdownPreview,
      runPrompt: (text) => runPromptRef.current(text),
    }))
    crepe.editor.use($prose(() => activeBlockPlugin))
    crepe.editor.use($prose(() => createBlockSelectionPlugin()))
    crepe.editor.use($prose(() => createPaginationPlugin({
      getGeometry: () => paginationGeometryRef.current,
      getContainer: () => pagesRef.current,
      onPageCountChange: (count) => {
        if (isMounted) setPageCount(count)
      },
      onFlowBreaksChange: (breaks) => {
        if (!isMounted) return
        setFlowBreaks(breaks)
        setPagesLaidOut(paginationGeometryRef.current !== null)
      },
    })))
    crepe.editor.use($prose(() => createFormatToolbarPlugin({
      onChange: (state) => {
        if (isMounted) setFormatToolbarState(state)
      },
      isInsideToolbar: (node) => Boolean(node && toolbarRef.current?.contains(node)),
    })))

    crepe.editor.use(
      createWikiLinkPlugin({
        getLookup: () => wikiLinkLookupRef.current,
        isMenuOpen: () => isWikiLinkMenuOpenRef.current,
        onMenuContextChange: (context) => {
          // The backend ranks the suggestions; only the latest request is shown.
          const request = ++wikiLinkSuggestionRequestRef.current
          const currentLibraryId = libraryIdRef.current
          if (!context || !currentLibraryId) {
            setWikiLinkMenuState(null)
            return
          }
          void suggestLinkTargets(currentLibraryId, context.query)
            .catch(() => [])
            .then((suggestions) => {
              if (request !== wikiLinkSuggestionRequestRef.current) return
              setWikiLinkMenuState((currentState) => {
                const nextState = buildWikiLinkMenuState(context, currentState, suggestions)
                return isSameMenuState(currentState, nextState) ? currentState : nextState
              })
            })
        },
        onMoveMenuSelection: (direction) => {
          setWikiLinkMenuState((currentState) => {
            if (!currentState || currentState.suggestions.length === 0) {
              return currentState
            }

            const optionsCount = currentState.suggestions.length
            const nextIndex =
              direction === 1
                ? (currentState.selectedIndex + 1) % optionsCount
                : (currentState.selectedIndex - 1 + optionsCount) % optionsCount

            if (nextIndex === currentState.selectedIndex) {
              return currentState
            }

            return {
              ...currentState,
              selectedIndex: nextIndex,
            }
          })
        },
        onCloseMenu: () => {
          setWikiLinkMenuState((currentState) => (currentState ? null : currentState))
        },
        onConfirmMenuSelection: (view) => {
          const menuState = wikiLinkMenuStateRef.current
          if (!menuState || menuState.suggestions.length === 0) {
            return false
          }

          const target = menuState.suggestions[menuState.selectedIndex] ?? menuState.suggestions[0]
          if (!target) {
            return false
          }

          insertWikiLinkSuggestion(view, menuState, target)
          setWikiLinkMenuState(null)
          return true
        },
        onOpenLinkPath: (path) => {
          onOpenLinkedFileRef.current(path)
        },
      }),
    )

    crepe.editor.use(gfm)
    crepe.editor.use(tableCellBlocksRemark)
    crepe.editor.use(markdownTableBlockView)
    crepe.editor.use(emoji)
    crepe.editor.use(cursor)
    crepe.editor.use(indent)
    crepe.editor.use(trailing)
    crepe.editor.use($prose(() => createBlockGapPlugin()))
    crepe.editor.use(clipboard)

    const handleEditorMarkdown = (markdown: string) => {
      if (!isMounted || isApplyingExternalUpdateRef.current) {
        return
      }

      if (markdown === latestBodyRef.current) {
        return
      }

      latestBodyRef.current = markdown

      const nextSource = serializeFrontmatterDocument({
        hasFrontmatter: hasFrontmatterRef.current,
        frontmatter: frontmatterRef.current,
        body: markdown,
      })

      if (nextSource === latestComposedSourceRef.current) {
        return
      }

      latestComposedSourceRef.current = nextSource
      onSourceChangeRef.current(nextSource, documentPathRef.current)
    }

    // The note's Markdown is written once typing pauses, not per keystroke.
    const markdownChanges = createMarkdownChangeBuffer<ProseNode>({
      isSame: (left, right) => left === right || left.eq(right),
      serialize: (doc) => crepe.editor.action((ctx) => ctx.get(serializerCtx)(doc)),
      emit: handleEditorMarkdown,
    })
    crepe.editor.use($prose(() => createMarkdownChangePlugin(markdownChanges)))
    flushEditorChangesRef.current = markdownChanges.flush
    // Closing a tab, saving before leaving the library or the app: what is pending goes first.
    const unregisterPendingChanges = registerPendingEditorChanges(markdownChanges.flush)

    crepeRef.current = crepe

    void crepe.create().then(() => {
      if (!isMounted) {
        return
      }

      isReadyRef.current = true
      setIsEditorReady(true)
      followPenTool(crepe, penToolRef.current, readOnlyRef.current)
      addMathOcrButtons()
      addInlineLatexInkMathButtons()
      const editorView = crepe.editor.action((ctx) => ctx.get(editorViewCtx))
      const selectionNotifier = createSelectionNotifier({
        read: () => buildMarkdownSelectionContext(editorView.state, documentPathRef.current),
        emit: (selection) => onSelectionChangeRef.current(selection),
      })
      selectionNotifierRef.current = selectionNotifier
      const notifySelectionChange = selectionNotifier.schedule
      editorView.dom.addEventListener('keyup', notifySelectionChange)
      editorView.dom.addEventListener('mouseup', notifySelectionChange)
      editorView.dom.addEventListener('touchend', notifySelectionChange)
      document.addEventListener('selectionchange', notifySelectionChange)
      const root = rootRef.current
      const host = viewportRef.current
      const stopObservingHandle = root
        ? observeBlockHandleVisibility(root, () => setActiveBlock(editorView, null))
        : () => {}
      const detachDragGhost = root && host
        ? attachBlockDragGhost(root, host, () => {
          const { selection } = editorView.state
          return selection instanceof NodeSelection ? selection.node.textContent : ''
        })
        : () => {}
      const stopHidingHandle = root && host ? hideBlockHandleOnPointerLeave(host, root) : () => {}
      const stopTrackingPointerRow = trackPointerRow(editorView, (clientY) => {
        pointerRowRef.current = clientY
      })
      // The blank page under the note belongs to the host, not to the editor.
      const clickBelowContent = (event: MouseEvent) => {
        const pages = pagesRef.current
        const target = event.target
        const isOnPage = target === host || (target instanceof Node && zoomContentRef.current?.contains(target))
        if (pages && isOnPage) handleClickBelowContent(editorView, event, pages.getBoundingClientRect())
      }
      // With the selector, an empty place is the marquee's: a press there is
      // a click when it does not drag, and the marquee reports it. A finger,
      // or the stylus on Android, never starts the marquee, so its tap is
      // handled here (it is what adds the line under the last block).
      let marqueeTakesLastPress = true
      const onHostPointerDown = (event: PointerEvent) => {
        marqueeTakesLastPress = marqueeTakesPress(event)
      }
      const onHostMouseDown = (event: MouseEvent) => {
        if (host && marqueeTakesLastPress && penToolRef.current === 'selector' && isMarqueeTarget(event.target, host)) return
        clickBelowContent(event)
      }
      host?.addEventListener('pointerdown', onHostPointerDown, true)
      host?.addEventListener('mousedown', onHostMouseDown, true)
      const detachStylusWriting = host
        ? leaveStylusToSystem(host, editorView.dom, () => penToolRef.current === 'selector')
        : () => {}
      const detachMarquee = host
        ? attachBlockMarquee(host, {
          isEnabled: () => penToolRef.current === 'selector',
          getView: () => editorView,
          onClick: clickBelowContent,
        })
        : () => {}
      selectionCleanupRef.current = () => {
        editorView.dom.removeEventListener('keyup', notifySelectionChange)
        editorView.dom.removeEventListener('mouseup', notifySelectionChange)
        editorView.dom.removeEventListener('touchend', notifySelectionChange)
        document.removeEventListener('selectionchange', notifySelectionChange)
        stopObservingHandle()
        detachDragGhost()
        stopHidingHandle()
        stopTrackingPointerRow()
        host?.removeEventListener('pointerdown', onHostPointerDown, true)
        host?.removeEventListener('mousedown', onHostMouseDown, true)
        detachStylusWriting()
        detachMarquee()
      }
      selectionNotifier.flush()
    })

    return () => {
      // The last keystrokes go to their note before the editor closes.
      markdownChanges.flush()
      unregisterPendingChanges()
      flushEditorChangesRef.current = () => {}
      isMounted = false
      isReadyRef.current = false
      crepeRef.current = null
      setWikiLinkMenuState(null)
      selectionNotifierRef.current?.cancel()
      selectionNotifierRef.current = null
      onSelectionChangeRef.current(null)
      if (chromeFrame !== null) window.cancelAnimationFrame(chromeFrame)
      codeBlockObserver.disconnect()
      cleanupXGraphPreviews()
      gitbookResolver.dispose()
      if (gitbookResolverRef.current === gitbookResolver) gitbookResolverRef.current = null
      cleanupInlinePreviews()
      clearDocumentRefs()
      selectionCleanupRef.current?.()
      selectionCleanupRef.current = null
      void crepe.destroy()
    }
  }, [])

  useEffect(() => {
    if (!source) return
    mermaidPreviewBlockIndexRef.current = 0
  }, [source])


  useEffect(() => {
    const crepe = crepeRef.current
    if (!crepe || !isReadyRef.current || !isEditorReady || !externalSourceUpdate) {
      return
    }

    if (externalSourceUpdate.documentPath !== documentPath) {
      return
    }

    const nextDocument = parseFrontmatterDocument(externalSourceUpdate.source)
    isApplyingExternalUpdateRef.current = true
    crepe.editor.action(replaceAll(nextDocument.body, true))
    isApplyingExternalUpdateRef.current = false
    latestComposedSourceRef.current = externalSourceUpdate.source
    latestBodyRef.current = nextDocument.body
    frontmatterRef.current = nextDocument.frontmatter
    hasFrontmatterRef.current = nextDocument.hasFrontmatter

    const editorView = crepe.editor.action((ctx) => ctx.get(editorViewCtx))
    const selection = buildMarkdownSelectionContext(editorView.state, documentPath)
    if (selectionNotifierRef.current) selectionNotifierRef.current.sendNow(selection)
    else onSelectionChangeRef.current(selection)
  }, [documentPath, externalSourceUpdate, isEditorReady])

  useEffect(() => {
    const crepe = crepeRef.current
    if (!crepe || !isReadyRef.current) {
      return
    }

    // When props are behind local typing, do not push stale content into the editor.
    if (source !== latestComposedSourceRef.current) {
      return
    }

    // If our tracked body already matches the incoming body, this is not an external body change.
    if (parsedDocument.body === latestBodyRef.current) {
      return
    }

    const currentMarkdown = crepe.getMarkdown()
    if (currentMarkdown === parsedDocument.body) {
      latestBodyRef.current = currentMarkdown
      return
    }

    isApplyingExternalUpdateRef.current = true
    crepe.editor.action(replaceAll(parsedDocument.body, true))
    isApplyingExternalUpdateRef.current = false
    latestBodyRef.current = parsedDocument.body
  }, [parsedDocument.body, source])

  useEffect(() => {
    const crepe = crepeRef.current
    if (!crepe || !isReadyRef.current) {
      return
    }

    crepe.editor.action((ctx) => {
      const view = ctx.get(editorViewCtx)
      const refreshTransaction = view.state.tr.setMeta('notia-refresh-wikilinks', Date.now())
      view.dispatch(refreshTransaction)
    })
  }, [wikiLinkLookup])

  // Stable, so the properties panel is not drawn again while the body is typed.
  const handleAddProperty = useCallback((entry: FrontmatterEntry) => {
    // The body the editor still holds goes with the new property.
    flushEditorChangesRef.current()
    const nextEntries = [...frontmatterRef.current, entry]
    frontmatterRef.current = nextEntries
    hasFrontmatterRef.current = true

    const nextSource = serializeFrontmatterDocument({
      hasFrontmatter: true,
      frontmatter: nextEntries,
      body: latestBodyRef.current,
    })

    latestComposedSourceRef.current = nextSource
    onSourceChangeRef.current(nextSource, documentPathRef.current)
  }, [])

  const handleEditProperty = useCallback(async (key: string, value: unknown) => {
    flushEditorChangesRef.current()
    const index = frontmatterRef.current.findIndex((entry) => entry.key === key)
    if (index < 0) return

    // Capture the old value BEFORE mutating.
    const oldValue = frontmatterRef.current[index]?.value

    const nextEntries = [...frontmatterRef.current]
    nextEntries[index] = { key, value: value as FrontmatterEntry['value'] }
    frontmatterRef.current = nextEntries

    const nextSource = serializeFrontmatterDocument({
      hasFrontmatter: true,
      frontmatter: nextEntries,
      body: latestBodyRef.current,
    })

    latestComposedSourceRef.current = nextSource
    onSourceChangeRef.current(nextSource, documentPath)

    // Page links are bidirectional: the backend checks cycles and updates
    // the opposite link of the previous and new target notes.
    const lowerKey = key.toLowerCase()
    if ((lowerKey === 'nextpage' || lowerKey === 'previouspage') && libraryId) {
      try {
        const result = await callBackend<{ currentValue: string; changed: boolean; error?: string | null }>('backend_sync_page_link', {
          payload: {
            libraryId,
            logicalPath: documentPath,
            key: lowerKey === 'nextpage' ? 'nextPage' : 'previousPage',
            oldValue: typeof oldValue === 'string' ? oldValue : '',
            newValue: typeof value === 'string' ? value : '',
          },
        })
        if (result.currentValue !== value) {
          const entries = frontmatterRef.current.map((entry) => (
            entry.key === key ? { key, value: result.currentValue as FrontmatterEntry['value'] } : entry
          ))
          frontmatterRef.current = entries
          const canonicalSource = serializeFrontmatterDocument({
            hasFrontmatter: true,
            frontmatter: entries,
            body: latestBodyRef.current,
          })
          latestComposedSourceRef.current = canonicalSource
          onSourceChangeRef.current(canonicalSource, documentPath)
        }
        if (result.error) {
          console.error('[MarkdownView] Page link sync error:', result.error)
        }
      } catch (error) {
        console.error('[MarkdownView] Page link sync error:', error)
      }
    }
  }, [documentPath, libraryId])

  const handleDeleteProperty = useCallback((key: string) => {
    flushEditorChangesRef.current()
    const nextEntries = frontmatterRef.current.filter((entry) => entry.key !== key)
    frontmatterRef.current = nextEntries
    if (nextEntries.length === 0) {
      hasFrontmatterRef.current = false
    }

    const nextSource = serializeFrontmatterDocument({
      hasFrontmatter: hasFrontmatterRef.current,
      frontmatter: nextEntries,
      body: latestBodyRef.current,
    })

    latestComposedSourceRef.current = nextSource
    onSourceChangeRef.current(nextSource, documentPathRef.current)
  }, [])

  /** Runs a toolbar command; on a block's toolbar it first selects that block's text. */
  const runFormat = (action: (view: EditorView) => void) => {
    const crepe = crepeRef.current
    if (!crepe || !isReadyRef.current) return
    const view = crepe.editor.action((ctx) => ctx.get(editorViewCtx))
    const target = formatToolbarState?.target
    if (target?.kind === 'block') selectBlockText(view, target.pos)
    action(view)
    view.focus()
  }

  const formatToolbarActions: MarkdownFormatToolbarActions = {
    onBlockKind: (kind) => runFormat((view) => setBlockKind(view, kind)),
    onToggleMark: (name) => runFormat((view) => toggleFormatMark(view, name)),
    onColor: (name, color) => runFormat((view) => setColorMark(view, name, color)),
    onAlign: (align) => runFormat((view) => setBlockAlignment(view, align)),
    // Crepe's link tooltip asks for the address.
    onLink: () => runFormat(() => crepeRef.current?.editor.action((ctx) => ctx.get(commandsCtx).call(toggleLinkCommand.key))),
    onClear: () => runFormat(clearFormatting),
    // Inline elements go at the selection, not over a whole block.
    onInsertInline: (kind) => {
      const crepe = crepeRef.current
      if (!crepe || !isReadyRef.current) return
      insertGitbookInline(crepe.editor.action((ctx) => ctx.get(editorViewCtx)), kind)
    },
  }

  const handleWikiLinkSelect = (index: number) => {
    const menuState = wikiLinkMenuStateRef.current
    const crepe = crepeRef.current
    if (!menuState || !crepe || !isReadyRef.current) {
      return
    }

    const target = menuState.suggestions[index]
    if (!target) {
      return
    }

    const view = crepe.editor.action((ctx) => ctx.get(editorViewCtx))
    insertWikiLinkSuggestion(view, menuState, target)
    setWikiLinkMenuState(null)
  }

  const pageStride = pagePixels ? pagePixels.height + PAGE_GAP_PX : 0
  // Pages: the text's, the ones the strokes reach (drawn on a page past the
  // text) and the ones scrolling added.
  const sheetCount = pagePixels
    ? Math.max(
      pageCount,
      lowestInk > 0 ? Math.floor(flowToPage(lowestInk, flowBreaks) / pageStride) + 1 : 1,
      Math.ceil((scrolledSheetLength + PAGE_GAP_PX) / pageStride),
    )
    : 0
  const pagesStyle = sheetPixels
    ? {
      '--notia-page-width': `${sheetPixels.width}px`,
      '--notia-page-height': `${sheetPixels.height}px`,
      '--notia-page-margin': `${sheetPixels.margin}px`,
      '--notia-page-number-bottom': `${Math.max(10, Math.round(sheetPixels.margin / 2 - 8))}px`,
      // The continuous sheet reaches its lowest stroke, with room below to
      // keep writing, and at least the bottom of the editor.
      minHeight: pagePixels
        ? sheetCount * pagePixels.height + (sheetCount - 1) * PAGE_GAP_PX
        : Math.max(
          Math.ceil(lowestInk) + (penTool === 'selector' ? INK_TAIL_PX : INK_DRAWING_TAIL_PX),
          Math.max(0, hostHeight - deskPadding) / sheetScale,
          scrolledSheetLength,
        ),
      transform: sheetScale === 1 ? undefined : `scale(${sheetScale})`,
    } as CSSProperties
    : undefined
  const hostMode = pagePixels ? ' is-sheet is-paged' : sheetPixels ? ' is-sheet is-continuous' : ''

  return (
    <div
      ref={viewportRef}
      className={`notia-markdown-host${hostMode}`}
      style={sheetPixels ? { '--notia-desk-padding': `${deskPadding}px` } as CSSProperties : undefined}
      aria-label="Markdown editor"
    >
      {sheetPixels && !readOnly ? (
        <MarkdownPenBar
          tool={penTool}
          onToolChange={setPenTool}
          pen={pen}
          onPenChange={updatePen}
          canUndo={ink.canUndo}
          canRedo={ink.canRedo}
          onUndo={ink.undo}
          onRedo={ink.redo}
          error={ink.error}
        />
      ) : null}
      <div ref={zoomContentRef} className="notia-markdown-zoom-content">
        {/* The structure is the same in both modes, so switching never remounts the editor. */}
        <SheetFrame pagesRef={pagesRef} sheetWidth={sheetPixels?.width ?? null} scale={sheetScale}>
        <div ref={pagesRef} className="notia-markdown-pages" style={pagesStyle}>
          {pagePixels ? (
            <div className="notia-markdown-page-sheets" aria-hidden="true">
              {Array.from({ length: sheetCount }, (_, index) => (
                <div key={index} className="notia-markdown-page-sheet" style={{ top: index * (pagePixels.height + PAGE_GAP_PX) }}>
                  {pagePixels.pageNumbers ? <span className="notia-markdown-page-number">{index + 1} / {sheetCount}</span> : null}
                </div>
              ))}
            </div>
          ) : sheetPixels ? (
            <div className="notia-markdown-page-sheets" aria-hidden="true">
              <div className="notia-markdown-page-sheet is-continuous" />
            </div>
          ) : null}
          <div className="notia-markdown-page-flow">
            <ChatAttachmentImages source={source} />
            {readOnly ? (
              <p className="notia-markdown-readonly" role="note">
                <Lock size={13} strokeWidth={2} aria-hidden="true" />
                <span>{readOnly}</span>
              </p>
            ) : null}
            <div className="notia-markdown-properties-wrap">
              <MarkdownPropertiesPanel
                entries={frontmatterEntries}
                wikiLinkLookup={wikiLinkLookup}
                libraryId={libraryId}
                onAddProperty={handleAddProperty}
                onEditProperty={handleEditProperty}
                onDeleteProperty={handleDeleteProperty}
                onOpenLinkedFile={onOpenLinkedFile}
                onCreateLinkedNote={onCreateLinkedNote}
                contexts={contexts}
                lockedContextTag={lockedContextTag}
                lockedContextSource={lockedContextSource}
                readOnly={Boolean(readOnly)}
              />
            </div>
            <div ref={rootRef} className="notia-markdown-editor-root" />
          </div>
          {sheetPixels && pen ? (
            <InkLayer
              strokes={ink.strokes}
              tool={penTool}
              pen={pen}
              sheet={inkSheet ?? { width: sheetPixels.width, paged: false, pageHeight: sheetPixels.height, pageGap: PAGE_GAP_PX, margin: sheetPixels.margin, numberBand: 0 }}
              breaks={flowBreaks}
              scrollContainerRef={viewportRef}
              onCommit={ink.commit}
              onErase={ink.erase}
              onLassoSelect={ink.select}
              onMove={ink.move}
              onRemove={ink.remove}
              onConvert={handleInkConvert}
            />
          ) : null}
        </div>
        </SheetFrame>
      </div>
      <WikiLinkSuggestionMenu state={wikiLinkMenuState} onSelect={handleWikiLinkSelect} />
      <WikiLinkPreviewCard rootRef={rootRef} libraryId={libraryId} />
      {readOnly ? null : (
        <MarkdownFormatToolbar
          state={formatToolbarState}
          hostRef={viewportRef}
          toolbarRef={toolbarRef}
          {...formatToolbarActions}
        />
      )}
    </div>
  )
}

export const MarkdownView = memo(MarkdownViewInner)
MarkdownView.displayName = 'MarkdownView'
