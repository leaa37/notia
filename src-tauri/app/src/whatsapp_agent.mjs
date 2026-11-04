// The WhatsApp link of Notia: Baileys connects the Owner's WhatsApp as one
// more linked device and this process tells Notia what happens, one JSON
// line per event on stdout, and runs what Notia asks, one JSON line per
// request on stdin, answered with its id. Notia keeps the chats and the
// messages, decides what may be read or sent and writes every text; this
// process only talks to WhatsApp.
//
//   node whatsapp_agent.mjs <data folder>
//
// The data folder holds the session (`auth`, the keys of the linked
// device) and the media messages whose files can still be downloaded
// (`media.json`). It ends when stdin closes (Notia stopped it), and after
// WhatsApp unlinks the device, with the session deleted.
//
// Events: { event: "qr", svg } · { event: "state", state, account?, reason? }
// (state: connecting, open, closed, loggedOut, replaced) ·
// { event: "chats", chats: [{ id, name?, timestampMs?, unread? }] } ·
// { event: "chatsDeleted", ids } · { event: "contacts", contacts: [{ id, name?, notify? }] } ·
// { event: "messages", live, messages: [{ chat, id, fromMe, sender?, senderName?,
//   timestampMs, text, kind, fileName?, options? }] } (options: what a bot's
//   menu offers to tap).
// Requests: send { chat, text, lookup? } · reply { chat, messageId, option }
// (taps an option of a menu) · sendFile { chat, base64, fileName, mimetype,
// caption?, asImage, lookup? } · download { messageId } · logout.
// With `lookup` the chat is a number nobody wrote to yet: WhatsApp says
// whether it has an account and its real id.

import fs from 'node:fs'
import path from 'node:path'
import readline from 'node:readline'
import makeWASocket, {
  Browsers,
  BufferJSON,
  DisconnectReason,
  downloadMediaMessage,
  fetchLatestBaileysVersion,
  generateWAMessageFromContent,
  getContentType,
  isJidBroadcast,
  isJidGroup,
  isJidNewsletter,
  isJidStatusBroadcast,
  isLidUser,
  jidNormalizedUser,
  normalizeMessageContent,
  useMultiFileAuthState,
} from 'baileys'
import QRCode from 'qrcode'

const dataFolder = process.argv[2]
const authFolder = path.join(dataFolder, 'auth')
const mediaFile = path.join(dataFolder, 'media.json')
const MAX_RECENT = 2000
const MAX_MEDIA = 500
const MAX_TEXT = 4000
const MAX_OPTIONS = 30
const MAX_OPTION_CHARS = 100
const RETRY_FIRST_MS = 3000
const RETRY_MAX_MS = 60000

// stdout carries the protocol only; what a library prints goes to stderr.
for (const name of ['log', 'info', 'warn', 'error', 'debug']) {
  console[name] = (...parts) => process.stderr.write(parts.map(String).join(' ') + '\n')
}

const silent = {
  level: 'silent',
  child() { return silent },
  trace() {},
  debug() {},
  info() {},
  warn() {},
  error() {},
  fatal() {},
}

function emit(value) {
  process.stdout.write(JSON.stringify(value) + '\n')
}

let socket = null
let open = false
let retryMs = RETRY_FIRST_MS
let ending = false
/** Phone ids of contacts WhatsApp names by their hidden id (`@lid`). */
const phoneOfLid = new Map()
/** Recent messages by id, for WhatsApp's resends and the downloads. */
const recent = new Map()
/** Media messages by id, kept on disk to download them after a restart. */
const media = loadMedia()
let mediaTimer = null

function remember(map, id, message, max) {
  map.delete(id)
  map.set(id, message)
  while (map.size > max) map.delete(map.keys().next().value)
}

function loadMedia() {
  try {
    const entries = JSON.parse(fs.readFileSync(mediaFile, 'utf8'), BufferJSON.reviver)
    return new Map(Array.isArray(entries) ? entries.slice(-MAX_MEDIA) : [])
  } catch {
    return new Map()
  }
}

function saveMediaSoon() {
  if (mediaTimer) return
  mediaTimer = setTimeout(() => {
    mediaTimer = null
    try {
      const temporary = `${mediaFile}.tmp`
      fs.writeFileSync(temporary, JSON.stringify([...media.entries()], BufferJSON.replacer))
      fs.renameSync(temporary, mediaFile)
    } catch {
      // The downloads of older images fail; the link goes on.
    }
  }, 2000)
}

function isChat(jid) {
  return Boolean(jid) && !isJidBroadcast(jid) && !isJidStatusBroadcast(jid) && !isJidNewsletter(jid)
}

function learnLid(lid, phone) {
  if (lid && phone && isLidUser(lid) && !isLidUser(phone)) phoneOfLid.set(jidNormalizedUser(lid), jidNormalizedUser(phone))
}

/** The id Notia keeps for a chat or a person: the phone one when known. */
function chatId(jid, alternative) {
  if (!jid) return null
  const id = jidNormalizedUser(jid)
  if (!isLidUser(id)) return id
  if (alternative && !isLidUser(alternative)) {
    learnLid(id, alternative)
    return jidNormalizedUser(alternative)
  }
  return phoneOfLid.get(id) ?? id
}

function cut(text) {
  return typeof text === 'string' ? text.slice(0, MAX_TEXT) : ''
}

function lines(...parts) {
  return parts.filter((part) => typeof part === 'string' && part.trim()).map((part) => part.trim()).join('\n')
}

function buttonParams(button) {
  try {
    return JSON.parse(button?.buttonParamsJson || '{}') ?? {}
  } catch {
    return {}
  }
}

/** What answering a native flow menu sends, as the phone does when its option is tapped. */
function flowReply(name, label, id) {
  return {
    interactiveResponseMessage: {
      body: { text: label, format: 0 },
      nativeFlowResponseMessage: { name, paramsJson: JSON.stringify({ id }), version: 3 },
    },
  }
}

/**
 * A bot's menu: its text, the options to tap with what tapping each sends,
 * and its links and phones (they open on the phone, nobody answers them);
 * `null` when the message is not a menu.
 */
function menuOf(content) {
  const type = getContentType(content)
  const inner = type ? content[type] : null
  const options = []
  const links = []
  const option = (label, reply) => {
    const text = typeof label === 'string' ? label.trim().slice(0, MAX_OPTION_CHARS) : ''
    if (text && options.length < MAX_OPTIONS) options.push({ label: text, reply })
  }
  let text
  switch (type) {
    case 'buttonsMessage':
      text = lines(inner?.text, inner?.contentText, inner?.footerText)
      ;(inner?.buttons ?? []).forEach((button, index) => {
        const label = button?.buttonText?.displayText
        option(label, { buttonReply: { displayText: label, id: button?.buttonId ?? '', index }, type: 'plain' })
      })
      break
    case 'templateMessage': {
      const template = inner?.hydratedTemplate ?? inner?.hydratedFourRowTemplate
      text = lines(template?.hydratedTitleText, template?.hydratedContentText, template?.hydratedFooterText)
      ;(template?.hydratedButtons ?? []).forEach((button, index) => {
        const quick = button?.quickReplyButton
        if (quick) option(quick.displayText, { buttonReply: { displayText: quick.displayText, id: quick.id ?? '', index: button.index ?? index }, type: 'template' })
        else if (button?.urlButton) links.push(lines(button.urlButton.displayText, button.urlButton.url).replace('\n', ': '))
        else if (button?.callButton) links.push(lines(button.callButton.displayText, button.callButton.phoneNumber).replace('\n', ': '))
      })
      break
    }
    case 'listMessage':
      text = lines(inner?.title, inner?.description, inner?.footerText)
      for (const section of inner?.sections ?? []) {
        for (const row of section?.rows ?? []) {
          option(row?.title, { listReply: { title: row?.title, description: row?.description ?? undefined, listType: 1, singleSelectReply: { selectedRowId: row?.rowId ?? '' } } })
        }
      }
      break
    case 'interactiveMessage':
      text = lines(inner?.header?.title, inner?.header?.subtitle, inner?.body?.text, inner?.footer?.text)
      for (const button of inner?.nativeFlowMessage?.buttons ?? []) {
        const params = buttonParams(button)
        if (button?.name === 'quick_reply') option(params.display_text, flowReply(button.name, params.display_text, params.id ?? ''))
        else if (button?.name === 'single_select') {
          for (const section of params.sections ?? []) {
            for (const row of section?.rows ?? []) option(row?.title, flowReply(button.name, row?.title, row?.id ?? ''))
          }
        } else if (button?.name === 'cta_url') links.push(lines(params.display_text, params.url).replace('\n', ': '))
        else if (button?.name === 'cta_call') links.push(lines(params.display_text, params.phone_number).replace('\n', ': '))
        else if (button?.name === 'cta_copy') links.push(lines(params.display_text, params.copy_code).replace('\n', ': '))
      }
      break
    default:
      return null
  }
  return { text: lines(text, links.length ? `Enlaces: ${links.join(' · ')}` : ''), options }
}

/** Kind, text and file name of a message; `null` for what is not a message to read (reactions, edits, deletions, keys). */
function describe(content) {
  const menu = menuOf(content)
  if (menu) return { kind: 'menu', text: cut(menu.text), options: menu.options.map((option) => option.label) }
  const type = getContentType(content)
  const inner = type ? content[type] : null
  switch (type) {
    case 'conversation': return { kind: 'text', text: cut(content.conversation) }
    case 'extendedTextMessage': return { kind: 'text', text: cut(inner?.text) }
    // What was tapped in a bot's menu.
    case 'buttonsResponseMessage':
    case 'templateButtonReplyMessage': return { kind: 'text', text: cut(inner?.selectedDisplayText) }
    case 'listResponseMessage': return { kind: 'text', text: cut(inner?.title) }
    case 'interactiveResponseMessage': return { kind: 'text', text: cut(inner?.body?.text) }
    case 'imageMessage': return { kind: 'image', text: cut(inner?.caption) }
    case 'videoMessage': return { kind: 'video', text: cut(inner?.caption) }
    case 'audioMessage': return { kind: 'audio', text: '' }
    case 'stickerMessage': return { kind: 'sticker', text: '' }
    case 'documentMessage': return { kind: 'document', text: cut(inner?.caption), fileName: inner?.fileName ?? undefined }
    case 'locationMessage':
    case 'liveLocationMessage': {
      const place = [inner?.name, inner?.address].filter(Boolean).join(', ')
      const where = inner?.degreesLatitude != null ? `${inner.degreesLatitude}, ${inner.degreesLongitude}` : ''
      return { kind: 'location', text: cut([place, where].filter(Boolean).join(' · ')) }
    }
    case 'contactMessage': return { kind: 'contact', text: cut(inner?.displayName) }
    case 'contactsArrayMessage': return { kind: 'contact', text: cut(inner?.displayName) }
    case 'pollCreationMessage':
    case 'pollCreationMessageV2':
    case 'pollCreationMessageV3': return { kind: 'poll', text: cut(inner?.name) }
    case 'reactionMessage':
    case 'protocolMessage':
    case 'senderKeyDistributionMessage':
    case 'pollUpdateMessage':
    case 'messageContextInfo':
    case undefined:
      return null
    default: return { kind: 'other', text: '' }
  }
}

function toWire(message) {
  const key = message?.key
  if (!key?.id || !isChat(key.remoteJid)) return null
  const content = normalizeMessageContent(message.message)
  if (!content) return null
  const described = describe(content)
  if (!described) return null
  const chat = chatId(key.remoteJid, key.remoteJidAlt)
  const group = isJidGroup(chat)
  const timestamp = Number(message.messageTimestamp ?? 0)
  remember(recent, key.id, message, MAX_RECENT)
  if (['image', 'sticker', 'video', 'audio', 'document'].includes(described.kind)) {
    remember(media, key.id, message, MAX_MEDIA)
    saveMediaSoon()
  }
  return {
    chat,
    id: key.id,
    fromMe: Boolean(key.fromMe),
    sender: group && key.participant ? chatId(key.participant, key.participantAlt) : undefined,
    senderName: key.fromMe ? undefined : (message.pushName ?? undefined),
    timestampMs: Number.isFinite(timestamp) ? timestamp * 1000 : 0,
    ...described,
  }
}

function emitMessages(messages, live) {
  const wire = messages.map(toWire).filter(Boolean)
  if (wire.length) emit({ event: 'messages', live, messages: wire })
}

function emitChats(chats) {
  const wire = []
  for (const chat of chats) {
    if (!isChat(chat?.id)) continue
    learnLid(chat.lidJid, chat.pnJid)
    const id = chatId(chat.id, chat.pnJid)
    const timestamp = Number(chat.conversationTimestamp ?? chat.lastMessageRecvTimestamp ?? 0)
    wire.push({
      id,
      name: chat.name ?? chat.subject ?? chat.displayName ?? undefined,
      timestampMs: Number.isFinite(timestamp) && timestamp > 0 ? timestamp * 1000 : undefined,
      unread: typeof chat.unreadCount === 'number' ? Math.max(0, chat.unreadCount) : undefined,
    })
  }
  if (wire.length) emit({ event: 'chats', chats: wire })
}

function emitContacts(contacts) {
  const wire = []
  for (const contact of contacts) {
    if (!contact?.id) continue
    learnLid(contact.lid ?? contact.id, contact.phoneNumber)
    wire.push({ id: chatId(contact.id, contact.phoneNumber), name: contact.name ?? undefined, notify: contact.notify ?? contact.verifiedName ?? undefined })
  }
  if (wire.length) emit({ event: 'contacts', contacts: wire })
}

async function emitGroups() {
  try {
    const groups = await socket.groupFetchAllParticipating()
    emitChats(Object.values(groups).map((group) => ({ id: group.id, subject: group.subject })))
  } catch {
    // The group names arrive later with their chats.
  }
}

function clearSession() {
  fs.rmSync(authFolder, { recursive: true, force: true })
  fs.rmSync(mediaFile, { force: true })
}

function finish(state) {
  ending = true
  emit({ event: 'state', state })
  setTimeout(() => process.exit(0), 200)
}

async function connect() {
  const { state, saveCreds } = await useMultiFileAuthState(authFolder)
  let version
  try {
    version = (await fetchLatestBaileysVersion()).version
  } catch {
    // Baileys' own version still connects.
  }
  socket = makeWASocket({
    auth: state,
    logger: silent,
    browser: Browsers.windows('Notia'),
    // The phone keeps getting its notifications.
    markOnlineOnConnect: false,
    syncFullHistory: false,
    getMessage: async (key) => recent.get(key.id)?.message ?? media.get(key.id)?.message ?? undefined,
    ...(version ? { version } : {}),
  })
  const current = socket
  current.ev.process(async (events) => {
    if (current !== socket) return
    if (events['creds.update']) await saveCreds()
    const update = events['connection.update']
    if (update) await onConnection(update)
    for (const mapping of [events['lid-mapping.update']].filter(Boolean)) learnLid(mapping.lid, mapping.pn)
    const history = events['messaging-history.set']
    if (history) {
      for (const mapping of history.lidPnMappings ?? []) learnLid(mapping.lid, mapping.pn)
      emitContacts(history.contacts ?? [])
      emitChats(history.chats ?? [])
      emitMessages(history.messages ?? [], false)
    }
    if (events['contacts.upsert']) emitContacts(events['contacts.upsert'])
    if (events['contacts.update']) emitContacts(events['contacts.update'])
    if (events['chats.upsert']) emitChats(events['chats.upsert'])
    if (events['chats.update']) emitChats(events['chats.update'])
    if (events['groups.upsert']) emitChats(events['groups.upsert'].map((group) => ({ id: group.id, subject: group.subject })))
    if (events['groups.update']) emitChats(events['groups.update'].filter((group) => group.id).map((group) => ({ id: group.id, subject: group.subject })))
    if (events['chats.delete']) emit({ event: 'chatsDeleted', ids: events['chats.delete'].filter(isChat).map((id) => chatId(id)) })
    const upsert = events['messages.upsert']
    if (upsert) emitMessages(upsert.messages ?? [], upsert.type === 'notify')
  })
}

async function onConnection(update) {
  if (update.qr) {
    try {
      emit({ event: 'qr', svg: await QRCode.toString(update.qr, { type: 'svg', margin: 1 }) })
    } catch {
      emit({ event: 'state', state: 'connecting', reason: 'No se pudo dibujar el código QR.' })
    }
  }
  if (update.connection === 'connecting') emit({ event: 'state', state: 'connecting' })
  if (update.connection === 'open') {
    open = true
    retryMs = RETRY_FIRST_MS
    const user = socket.user ?? {}
    emit({ event: 'state', state: 'open', account: { id: user.id ? jidNormalizedUser(user.id) : null, name: user.name ?? user.verifiedName ?? null } })
    await emitGroups()
  }
  if (update.connection === 'close') {
    open = false
    const code = update.lastDisconnect?.error?.output?.statusCode
    if (ending) return
    if (code === DisconnectReason.loggedOut) {
      clearSession()
      finish('loggedOut')
      return
    }
    if (code === DisconnectReason.connectionReplaced) {
      finish('replaced')
      return
    }
    // After the QR is scanned WhatsApp asks for a new connection right away.
    const wait = code === DisconnectReason.restartRequired ? 0 : retryMs
    retryMs = Math.min(retryMs * 2, RETRY_MAX_MS)
    emit({ event: 'state', state: 'closed', reason: code ? `código ${code}` : undefined })
    setTimeout(() => { connect().catch(() => finish('closed')) }, wait)
  }
}

function requireOpen() {
  if (!socket || !open) throw new Error('WhatsApp no está conectado ahora; probá de nuevo en un momento.')
}

/** The chat a message goes to; a new number is checked first. */
async function target(request) {
  if (!request.lookup) return request.chat
  const [found] = (await socket.onWhatsApp(request.chat)) ?? []
  if (!found?.exists) throw new Error('Ese número no tiene WhatsApp: revisá el código de país y el número.')
  return jidNormalizedUser(found.jid)
}

function sentAnswer(sent) {
  const message = sent ? toWire(sent) : null
  return { message: message ? { ...message, byNotia: true } : null }
}

async function handle(request) {
  switch (request.op) {
    case 'send': {
      requireOpen()
      return sentAnswer(await socket.sendMessage(await target(request), { text: request.text }))
    }
    case 'reply': {
      requireOpen()
      const menu = recent.get(request.messageId)
      if (!menu) throw new Error('Ese menú llegó antes de que Notia se conectara esta vez: mandá la opción como texto, sin replyTo.')
      const choice = menuOf(normalizeMessageContent(menu.message))?.options.find((option) => option.label === request.option)
      if (!choice) throw new Error('Esa opción ya no está en el menú: leé el chat de nuevo.')
      if (!choice.reply.interactiveResponseMessage) return sentAnswer(await socket.sendMessage(request.chat, choice.reply, { quoted: menu }))
      const message = generateWAMessageFromContent(request.chat, choice.reply, { userJid: socket.user.id, quoted: menu })
      await socket.relayMessage(request.chat, message.message, { messageId: message.key.id })
      return sentAnswer(message)
    }
    case 'sendFile': {
      requireOpen()
      const file = Buffer.from(request.base64, 'base64')
      const content = request.asImage
        ? { image: file, mimetype: request.mimetype, caption: request.caption || undefined }
        : { document: file, mimetype: request.mimetype, fileName: request.fileName, caption: request.caption || undefined }
      return sentAnswer(await socket.sendMessage(await target(request), content))
    }
    case 'download': {
      requireOpen()
      const message = media.get(request.messageId) ?? recent.get(request.messageId)
      if (!message) throw new Error('La imagen ya no se puede descargar: llegó antes de que Notia la guardara.')
      const content = normalizeMessageContent(message.message)
      const type = getContentType(content)
      const buffer = await downloadMediaMessage(message, 'buffer', {}, { logger: silent, reuploadRequest: socket.updateMediaMessage })
      return { base64: buffer.toString('base64'), mimetype: content?.[type]?.mimetype ?? 'application/octet-stream', bytes: buffer.length }
    }
    case 'logout': {
      ending = true
      try {
        if (socket) await socket.logout()
      } catch {
        // The session is deleted anyway; the phone shows the device until it notices.
      }
      clearSession()
      setTimeout(() => process.exit(0), 200)
      return {}
    }
    default:
      throw new Error(`Pedido desconocido: ${request.op}`)
  }
}

readline.createInterface({ input: process.stdin }).on('line', async (line) => {
  let request
  try {
    request = JSON.parse(line)
  } catch {
    return
  }
  try {
    emit({ id: request.id, ok: true, ...(await handle(request)) })
  } catch (error) {
    emit({ id: request.id, ok: false, error: String(error?.message ?? error).split('\n')[0].slice(0, 300) })
  }
}).on('close', () => process.exit(0))

connect().catch((error) => {
  emit({ event: 'state', state: 'closed', reason: String(error?.message ?? error).split('\n')[0].slice(0, 300) })
  process.exit(1)
})
