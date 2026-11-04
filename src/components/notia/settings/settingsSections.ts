import {
  Archive,
  AtSign,
  CloudSun,
  CreditCard,
  Globe,
  ImageIcon,
  Info,
  MessageCircle,
  Mic,
  PanelRight,
  PenLine,
  Send,
  Shield,
  Sparkles,
  Tag,
  Users,
  Webhook,
  type LucideIcon,
} from 'lucide-react'

export type SettingsSection = 'General' | 'Contextos' | 'Clima' | 'Roles' | 'Usuarios' | 'Panel desplegable' | 'InkMath' | 'IA' | 'Imágenes' | 'Voz' | 'Telegram' | 'WhatsApp' | 'Webhook' | 'Cuentas asociadas' | 'Finanzas' | 'Backups' | 'Publicar'

export const SETTINGS_SECTIONS: SettingsSection[] = ['General', 'Contextos', 'Clima', 'Roles', 'Usuarios', 'Panel desplegable', 'InkMath', 'IA', 'Imágenes', 'Voz', 'Telegram', 'WhatsApp', 'Webhook', 'Cuentas asociadas', 'Finanzas', 'Backups', 'Publicar']

export const SETTINGS_GROUPS: Array<{ label: string; sections: SettingsSection[] }> = [
  { label: 'Biblioteca', sections: ['General', 'Contextos', 'Clima'] },
  { label: 'Acceso', sections: ['Usuarios', 'Roles'] },
  { label: 'Editor', sections: ['Panel desplegable', 'InkMath'] },
  { label: 'Integraciones', sections: ['IA', 'Imágenes', 'Voz', 'Telegram', 'WhatsApp', 'Webhook', 'Cuentas asociadas'] },
  { label: 'Datos', sections: ['Backups', 'Publicar', 'Finanzas'] },
]

interface SettingsSectionMeta {
  description: string
  icon: LucideIcon
  /** Extra words the search matches besides the title and description. */
  keywords: string
}

export const SETTINGS_SECTION_META: Record<SettingsSection, SettingsSectionMeta> = {
  General: { description: 'Versión, dispositivo y biblioteca activa.', icon: Info, keywords: 'version plataforma' },
  Contextos: { description: 'Etiquetas que cada nota declara en su propiedad contexto. El color se usa en Graph View.', icon: Tag, keywords: 'tags color etiqueta' },
  Clima: { description: 'Lugar del pronóstico del Inicio y del asistente.', icon: CloudSun, keywords: 'tiempo pronostico ciudad lugar temperatura open-meteo' },
  Usuarios: { description: 'Quién entra a la biblioteca activa, con qué rol y qué contextos ve.', icon: Users, keywords: 'contraseña password permisos' },
  Roles: { description: 'Roles disponibles en la biblioteca activa.', icon: Shield, keywords: 'owner family guest' },
  'Panel desplegable': { description: 'Cada cuánto el panel busca cambios en la biblioteca.', icon: PanelRight, keywords: 'explorador chequeo cooldown refresco' },
  InkMath: { description: 'Reconocimiento de fórmulas manuscritas con Ollama.', icon: PenLine, keywords: 'ocr debounce formula' },
  IA: { description: 'Conexión con Ollama, modelo y cómo informa su progreso el agente.', icon: Sparkles, keywords: 'ollama api key modelo thinking host' },
  'Imágenes': { description: 'Modelo local con el que el asistente genera y edita imágenes.', icon: ImageIcon, keywords: 'imagen foto dibujo qwen image generar editar gpu modelo descargar' },
  Voz: { description: 'Reconocimiento y síntesis de voz locales.', icon: Mic, keywords: 'dictado whisper qwen tts idioma' },
  Telegram: { description: 'Bot de esta biblioteca y usuarios vinculados.', icon: Send, keywords: 'bot token' },
  WhatsApp: { description: 'Tu WhatsApp vinculado: qué chats lee y escribe el asistente y cuáles responde solo.', icon: MessageCircle, keywords: 'whatsapp wpp chat mensajes autorespuesta prompt baileys qr' },
  Webhook: { description: 'Acciones que otros programas pueden pedirle a Notia; la respuesta llega por Telegram.', icon: Webhook, keywords: 'webhook api http post endpoint accion prompt token alerta programa integracion' },
  'Cuentas asociadas': { description: 'Credenciales de Google Cloud y cuenta de Gmail de la biblioteca activa.', icon: AtSign, keywords: 'gmail correo mail email google cloud gcp client id secret oauth' },
  Backups: { description: 'Copias automáticas de la biblioteca activa.', icon: Archive, keywords: 'copia zip carpeta' },
  Publicar: { description: 'Compartí tableros del Task Manager y espacios públicos de lectura en la red local.', icon: Globe, keywords: 'task manager puerto navegador red espacios públicos gitbook sitio' },
  Finanzas: { description: 'Datos del módulo Finanzas en la biblioteca activa.', icon: CreditCard, keywords: 'eliminar borrar datos' },
}

export function settingsGroupOf(section: SettingsSection): string {
  return SETTINGS_GROUPS.find((group) => group.sections.includes(section))?.label ?? ''
}

function normalizeSearchText(value: string): string {
  return value.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().trim()
}

/** Visual filter of the navigation: every word of the query must appear. */
export function matchesSettingsSearch(section: SettingsSection, query: string): boolean {
  const words = normalizeSearchText(query).split(/\s+/).filter(Boolean)
  if (words.length === 0) return true
  const meta = SETTINGS_SECTION_META[section]
  const haystack = normalizeSearchText(`${section} ${settingsGroupOf(section)} ${meta.description} ${meta.keywords}`)
  return words.every((word) => haystack.includes(word))
}
