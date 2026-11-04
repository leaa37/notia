# Notia

Si el agente anuncia una acción —por ejemplo, «Ahora insertaré este gráfico»— y no llama a una herramienta, Notia le pide continuar dentro de la misma operación. Conserva las confirmaciones de escritura. Tras dos intentos de corrección sin ejecución, muestra un error; una promesa no confirma que el archivo haya cambiado. Esta recuperación reconoce anuncios explícitos de acción y no garantiza que cualquier modelo complete todos los pedidos.

El agente no muestra reglas internas, prompts, correcciones de validación ni nombres internos de herramientas. Si una respuesta del modelo intenta exponerlos, Notia la descarta y solicita una respuesta segura antes de enviarla.

Si el modo desarrollo reinicia repetidamente con mensajes `File ... .gradle/... changed. Rebuilding application`, reiniciá una vez `npm run dev:tauri:windows` para que lea `src-tauri/.taurignore`. Ese archivo excluye las cachés y salidas de Gradle y de compilación nativa tanto del proyecto Android como de las dependencias en `vendor/`. Los warnings de funciones Rust sin uso no son la causa.

## XGraph en notas Markdown

Si el código falla, una franja indica que el gráfico puede estar incompleto y conserva visible la parte construida. **Ver detalle del error** permite desplegar el mensaje técnico con toque o teclado. Corregí el código para actualizar el gráfico y retirar el aviso.

El agente de IA conoce XGraph y puede ayudarte a crear o corregir sus bloques. Por ejemplo: «Agregá en esta nota un gráfico XGraph de seno con un control de amplitud». Usa las herramientas y confirmaciones disponibles en el contexto actual. Esta guía llega también a agentes con prompts personalizados o bibliotecas existentes, sin sobrescribir sus instrucciones. El gráfico se visualiza al abrir la nota en Milkdown.

En una nota `.md`, abrí el menú de bloques de Milkdown (con `/` o el botón de agregar bloque) y elegí **XGraph**. Escribí JavaScript de JSXGraph: `board` es un tablero ya inicializado, con ejes y límites de −5 a 5. La vista previa se actualiza al editar, sobre un tablero con los colores del tema. **Ocultar código** deja solo el gráfico y **Mostrar** vuelve a mostrarlo, igual que en las fórmulas; **PNG** guarda el gráfico como imagen junto a la nota. Todos los controles admiten toque.

También podés escribir el bloque directamente:

```xgraph
board.create('point', [1, 2], { name: 'A' });
board.create('functiongraph', [(x) => Math.sin(x)]);
```

Se admite también el lenguaje `jsxgraph`. El código queda guardado en el `.md`; los movimientos interactivos del gráfico y el estado de Ocultar código/Mostrar son temporales. Para cambiar la escala usá `board.setBoundingBox([-10, 10, 10, -10])`. `JXG` y `BOARDID` también están disponibles; no hace falta pegar HTML ni importar scripts. Ver los ejemplos de la [documentación de JSXGraph](https://jsxgraph.org/home/start/gettingstarted/).

Funciona sin conexión. Cada gráfico se ejecuta aislado de Notia y no puede usar sus archivos, APIs ni realizar peticiones de red. Los errores aparecen en el visualizador; corregí el código para reintentar. El límite es de 100.000 caracteres por bloque. Evitá bucles infinitos o construcciones enormes: el aislamiento no impone un presupuesto de CPU. La exportación a PDF/Word conserva estos bloques como código, sin capturar el gráfico interactivo.

En el chat de **Finanzas** y en Telegram, el asistente es quien carga y corrige los datos financieros: la pantalla de Finanzas solo muestra. Podés mandarle la foto de un ticket, un recibo de sueldo, una factura o el resumen de la tarjeta, o contarle un gasto, y lo guarda con herramientas tipadas: cuentas, categorías, movimientos, ahorro, tickets, sueldos, resúmenes, planes de cuotas y servicios. También puede corregir o eliminar cualquier registro, unificar o renombrar productos y comercios, vincular o desvincular un gasto con su línea del resumen, limpiar Finanzas o extraer documentos para revisión. Cada cambio muestra un resumen y pide confirmación individual (en Telegram, una sola) y solo se informa como hecho después de verificar lo guardado. Al guardar, Notia une solo lo que es claramente lo mismo —por ejemplo, un ticket pagado con tarjeta y la línea del resumen que lo pagó— y el asistente te lo cuenta en una línea, con la opción de deshacerlo; lo dudoso (dos tickets posibles para una línea, un producto con nombre parecido a otro, una cuota que no aparece) queda en **Para revisar** y el asistente te lo pregunta. Al registrar el pago de un servicio consulta primero los datos locales y no inventa servicios. También puede obtener cotizaciones actuales de dólar oficial, blue y tarjeta desde DolarApi, e IPC mensual e interanual y el historial del dólar oficial desde ArgentinaDatos; las respuestas que usan fuentes externas indican su origen y pueden fallar si no hay conexión.

Las consultas sobre datos locales de Finanzas no usan ni sugieren `search_web`: se resuelven con las herramientas financieras y el snapshot de la biblioteca. Esto incluye pedidos como «mis últimos sueldos» o «sueldos cargados», que consultan `list_finance_salaries` y muestran como máximo los tres recibos más recientes, ordenados por `paymentDate` y luego `period`; los rangos, años y comparaciones usan `from` y/o `to` inclusivos en formato `YYYY-MM`. Las preguntas sobre salarios e IPC conservan la lectura local como evidencia y consultan además los índices de ArgentinaDatos; si faltan períodos, campos o monedas compatibles, no completan ni mezclan datos, y las lecturas limitadas no se presentan como inventarios completos. Los escenarios de factibilidad —por ejemplo, alquilar pagando un importe y ahorrar USD por mes con el sueldo— también conservan los salarios como evidencia intermedia y orientan la consulta a continuar con el dashboard financiero y, si hace falta convertir el objetivo, las cotizaciones del dólar; no afirman viabilidad si faltan gastos, compromisos, saldos o cotizaciones necesarios. En Telegram, el agente universal conserva el mismo enrutamiento cuando tiene habilitadas las herramientas de Finanzas. Las cotizaciones, el IPC y otros datos externos usan sus herramientas de consulta específicas; una consulta explícita de noticias o fuentes públicas puede usar búsqueda web sin confirmación adicional y se informa como tal. Las llamadas de herramienta idénticas dentro de una misma operación no se ejecutan nuevamente, y el límite predeterminado de rondas evita ciclos repetitivos sin cambiar las confirmaciones ni la autorización.

Las mutaciones de la biblioteca y de Finanzas muestran primero el cambio o preview, respetan autorización y confirmación, y solo se informan como exitosas después de verificar el resultado persistido. Las búsquedas públicas no modifican la biblioteca ni requieren confirmación adicional.

## Dictado offline en el chat

El contacto autorizado de Telegram también puede enviar una nota de voz OGG/Opus de hasta 15 minutos y 20 MB. Notia la descarga, la decodifica y carga bajo demanda el modelo offline necesario para transcribirla. La decodificación admite los modos SILK, CELT e híbrido usados por Telegram, incluso en notas de voz rápidas. Primero responde **Solicitud transcripción recibida y en proceso.**, mostrando la transcripción en negrita, y después procesa ese texto como una consulta normal del agente. El audio no se envía al proveedor de IA; Telegram sí interviene necesariamente en su transporte y descarga.

El contacto autorizado también puede enviar una foto de hasta 4 MB, una imagen JPG o PNG como archivo de hasta 10 MB o un PDF. Notia registra de forma durable tanto el update como la solicitud pendiente antes de procesarla, de modo que un reinicio del WebView reanuda la cola sin perder el documento, y la entrega al modelo de IA configurado junto con el contexto financiero local. Antes de responder, el asistente mira el adjunto y decide qué pedido es: un comprobante va a Finanzas, una captura de una agenda o un calendario se puede copiar a Google Calendar y unos apuntes se pueden guardar como nota. Si es un comprobante, lo clasifica como ticket de compra, recibo de sueldo, resumen de tarjeta de crédito u otro documento. Para tickets extrae comercio, fecha, productos, cantidades, precios y total, busca o crea la categoría y registra el gasto. Para recibos extrae período, fecha de cobro, empleador, bruto, descuentos, neto y conceptos, y registra tanto el recibo como el ingreso neto. Si la fuente es un PDF y el recibo indica que está firmado, conserva el neto impreso como valor autoritativo aunque difiera de bruto menos descuentos por adelantos, ajustes u otros conceptos de liquidación. Para resúmenes extrae emisor, tarjeta, período, vencimiento, saldos y líneas; registra consumos y cargos contra la cuenta de tarjeta en el mes del vencimiento, une los tickets y gastos con tarjeta que ya estaban cargados, concilia pagos y créditos y nunca cuenta el total a pagar como otro gasto. Las conciliaciones del resumen se consultan en los datos financieros locales. Solo pregunta si no puede identificar con suficiente claridad la cuenta correspondiente. Telegram informa los cambios de etapa y termina con un resultado verificable: documento registrado, duplicado o error concreto. Lo que mandás en un rato (varias fotos separadas, un álbum, el texto que las acompaña) se junta en un solo pedido, con hasta diez fotos o archivos, y la cola durable guarda hasta diez pedidos: el asistente procesa cada comprobante, cada uno con su confirmación, hasta terminar. Una foto que mandaste antes sigue a la vista del asistente en el mensaje donde la mandaste, así que podés preguntarle por ella después, pero ya no la toma como nueva en cada pedido: no vuelve a ofrecer cargar una comida o un ticket que ya registró. Esta función requiere un modelo configurado que admita imágenes y herramientas.

Cargar el resumen de la tarjeta significa que ya lo pagaste: cada consumo, cargo, interés e impuesto cuenta como gasto del mes en que vence el resumen, y el total es lo **pagado de tarjetas** de ese mes, nunca otro gasto; no hace falta registrar el pago aparte. Un ticket o un gasto pagado con tarjeta queda **en tarjeta, a pagar** hasta que llega su resumen, y no se cuenta dos veces: si el resumen ya estaba cargado, el ticket se une a su línea. Las líneas en cuotas arman solas el plan de cuotas de la compra, para ver lo que queda por pagar. Al guardar un resumen, las líneas `purchase` confirmadas también se comparan de forma determinista con el nombre o proveedor normalizado de los servicios. El matching acepta el servicio dentro de un descriptor con identificadores cuando aparece como palabra completa —por ejemplo, `MOVISTAR ARGENTINA 82997`—, pero no acepta coincidencias parciales como `Supermovistar`. Una línea se asigna al `statement.period`; si hay exactamente dos consumos del mismo servicio, se ordenan por fecha y se distribuyen entre el período anterior y el período del resumen. Si un consumo nombra más de un servicio, el asistente te pregunta de cuál es; los demás casos dudosos se corrigen pidiéndoselo en el chat.

Las asociaciones de una ocurrencia con un gasto comparan los importes por centavos, por lo que formatos equivalentes como `100` y `100.00` no se consideran cambios ni duplican el historial. Al vincularla, también se actualiza la compra asociada cuando existe. Un vínculo con moneda, tipo de transacción, importe o destino incompatible se rechaza.

Notia reconoce la voz localmente con Whisper large-v3-turbo (whisper.cpp), el único modelo de reconocimiento. En Windows usa la placa de video mediante Vulkan y, si el equipo no tiene un driver Vulkan, la CPU; en Android usa la CPU. Mientras hablás, el texto en vivo se va asentando: una palabra queda fija cuando dos lecturas seguidas del audio coinciden, así la vista previa no salta. Al terminar cada frase, Notia la vuelve a transcribir entera y guarda esa versión, la más precisa. En Android el modelo corre en el procesador y cada lectura tarda unos 10 segundos, así que no hay texto en vivo: las frases aparecen entre 10 y 30 segundos después de dichas, sin perder audio. En **Configuraciones → Voz** podés activarlo o desactivarlo y elegir el idioma en que transcribe, por ejemplo `es` (`es-AR` cuenta como `es`); con `auto`, Whisper lo detecta en cada frase. Si tenías el reconocimiento configurado con un modelo anterior, Notia conserva si estaba activo y su idioma.

Si el reconocimiento de voz está activo, Notia carga el modelo elegido en segundo plano apenas se inicia y lo mantiene en memoria mientras la aplicación está abierta, para que el dictado y Meeting empiecen a transcribir sin esperas. La ventana no espera esa carga: si abrís Meeting antes de que termine, verás «Preparando voz al iniciar Notia…» hasta que el modelo esté listo. Whisper queda listo en pocos segundos; la primera vez en Android tarda más, porque copia el modelo de unos 875 MB desde la aplicación. Qwen3-TTS se carga cuando se solicita una síntesis. Al cambiar el idioma, Notia prepara el reconocedor nuevo y descarta de forma segura el anterior.

El compositor incluye un botón de micrófono para dictar sin enviar audio ni texto a servicios externos. Cada respuesta del asistente muestra debajo del avatar una acción para leer ese mensaje con Qwen3-TTS usando la voz, el idioma y la velocidad configurados; una segunda pulsación detiene la reproducción. En Windows y Android arm64, Notia muestra texto parcial en tiempo real y, al detener, diariza el audio completo y vuelve a transcribir cada turno detectado para asignar el texto mediante sus límites temporales, separando intervenciones como `Hablante 1` y `Hablante 2` sin repartir palabras proporcionalmente. El resultado queda editable y nunca se envía automáticamente.

El acceso **Meeting**, ubicado debajo de ColdPass en el panel izquierdo, abre un espacio dedicado para grabar y transcribir reuniones, o transcribir una grabación que ya tengas, en cuatro pasos:

- **Lista para grabar:** el botón grande inicia la grabación (con teclado, `Ctrl + Shift + R`). Cada fuente tiene su interruptor: el micrófono y, en Windows, el audio de la computadora. **Probar audio** abre las fuentes solo para mostrar sus medidores, durante hasta dos minutos. También elegís el idioma, cuántos hablantes hay (o dejás que Notia lo detecte) y la carpeta de la biblioteca donde se guarda la nota; por defecto es `Meetings`. En **Contexto para la IA** elegís qué parte de tu biblioteca pueden consultar las respuestas en vivo y Notas IA: una carpeta (con sus subcarpetas) o **Toda la librería**, y en ese caso los **Contextos permitidos** (Todos, Ninguno o uno por uno; si hay muchos, **+N más** muestra el resto). Si apagás **Toda la librería**, la carpeta queda en **Ninguna** hasta que elijas una, y la IA no consulta tu biblioteca. La primera vez es toda la librería sin `#Confidencial`; después Notia recuerda lo que elegiste en cada biblioteca para la próxima grabación. En el celular está en la fila **Contexto IA**.
- **Reuniones anteriores:** a la derecha (en el celular, con el reloj de arriba) están las reuniones que guardaste como nota, de la más nueva a la más vieja, agrupadas en Hoy, Ayer, Esta semana y por mes. Cada una muestra su duración, cuántos hablantes tuvo, la carpeta o el contexto que consultó su IA y las tareas que quedan por mandar al Task Manager. Podés buscar por título o por lo que se dijo. Al tocar una, se abre finalizada, como la dejaste: podés seguir trabajando con ella y actualizar su nota. Una reunión que no guardaste no aparece; si borrás la nota, tampoco. También aparecen las que guardaste antes de que existiera esta lista: Notia las reconstruye desde su nota (fecha, duración, hablantes, resumen, Notas IA, marcas y transcripción).
- **Dispositivos (Windows):** debajo de cada fuente elegís qué micrófono y qué salida graba Notia; **Predeterminado de Windows** usa el de Windows. La elección se recuerda en este equipo y también vale para el dictado. Si el dispositivo es virtual (Voicemod, VB-Audio, Voicemeeter, NVIDIA Broadcast, OBS), Meeting avisa: ese audio llega procesado y baja la precisión de la transcripción, así que conviene elegir el micrófono real. Si la salida es de sonido envolvente (5.1 o 7.1, como muchos auriculares gamer), Notia toma solo los canales frontales y el central, donde está la voz.
- **Subir audio o video:** en la segunda pestaña podés elegir un archivo o soltarlo en la pantalla: audio (MP3, WAV, M4A, OGG, FLAC) o video (MP4, MOV, MKV, WEBM, del que se usa solo el audio), de hasta 2 GB. Notia muestra su duración, su tamaño y su onda. Con **Transcribir archivo** lo transcribe en tu equipo, igual que una grabación, con una barra de avance. Podés ver lo que ya transcribió o cancelar. Al terminar separa a los hablantes y la reunión queda como cualquier otra, titulada con el nombre del archivo.
- **Grabando:** muestra el tiempo, una onda por fuente y la transcripción en vivo con el minuto de cada frase. **Seguir en vivo** mantiene visible lo último. Podés pausar, reanudar, finalizar o cancelar. **Marcar momento** abre **Nueva marca**: escribís qué querés recordar y la marca queda en el minuto en que la abriste (vacía, toma las últimas palabras dichas). El panel de la derecha tiene dos pestañas:
  - **Notas IA** viene encendido: cada 5 minutos el agente lee la transcripción y las notas que ya tomó, y las reescribe y completa. Arma el objetivo, las decisiones, las preguntas abiertas, notas por tema (el actual dice «En curso») y tareas de seguimiento con responsable y plazo, que podés mandar **Al Task Manager**. Arriba se ve cuánto falta para la próxima actualización; **Llamar agente** la hace en el momento. También aparecen tus marcas, y **Agregar una nota propia…** guarda una nota tuya en ese minuto. Al detener la grabación, el agente hace una última pasada, y las notas quedan en la nota guardada.
  - **Respuestas en vivo** viene apagado: al activarlo, cuando alguien hace una pregunta, la IA sugiere una respuesta breve que podés copiar, pedir más corta o fijar a la nota; las anteriores quedan debajo.

  En el celular, el **Asistente en vivo** de abajo muestra el tema en curso con lo último de cada tipo y **Llamar agente**; el menú ⋮ abre **Notas IA completas**, las **Notas rápidas** y los momentos marcados.
- **Separando hablantes:** al finalizar, una barra muestra el avance y la etapa. Podés ver el texto sin separar o cancelar la separación y quedarte con la transcripción por minuto. En Windows con placa de video hay una **segunda pasada**: Notia vuelve a transcribir toda la reunión con un modelo más preciso (Whisper large-v3) y ese texto reemplaza al de la transcripción en vivo, frase por frase, sin cambiar quién la dijo. En la tablet o sin placa de video queda el texto en vivo.
- **Finalizada:** si configuraste la IA, apenas termina la separación la IA hace un repaso por su cuenta, y un aviso arriba dice qué está haciendo. Primero ordena la transcripción: corrige palabras mal oídas, saca muletillas («eeeh», «este»), repeticiones y arranques en falso, y deja cada intervención como un párrafo coherente, sin cambiar quién dijo qué. Después busca los nombres de los hablantes en la conversación (alguien se presenta, o le dan la palabra por su nombre y habla a continuación). Solo nombra a quien todavía se llama «Hablante N»: no toca los nombres que pusiste ni los de Teams. Mientras tanto, **Pasar por IA** espera. Arriba, **Tiempo de habla** muestra una línea de tiempo de quién habló en cada momento y, por hablante, su tiempo, su porcentaje, cuántas intervenciones tuvo, la más larga y el promedio. Podés renombrarlo con el lápiz o unir dos hablantes que Notia separó de más. La transcripción por turnos se puede buscar y filtrar por hablante. A la derecha, **Notas de la reunión** muestra las Notas IA (objetivo, decisiones, preguntas abiertas, tus marcas, notas por tema y tareas de seguimiento); tocar un tema te lleva a ese minuto de la transcripción. **Regenerar** le pide al agente que las vuelva a escribir con toda la transcripción (también si Notas IA estuvo apagado) y el botón de copiar las copia como texto. En **Preguntar a la IA**, **Pasar por IA** genera lo que elijas: resumen, puntos clave, tareas (que podés enviar a un tablero del Task Manager) y corrección de errores de dictado. En el celular son tres pestañas: Transcripción, Notas e IA. **Preguntale a la reunión** propone las preguntas que se hicieron en la reunión, cada una con su minuto, y responde indicando el minuto en que se dijo algo. **Guardar como nota** crea una nota en la carpeta elegida con el resumen, los puntos clave, las tareas, las Notas IA, tus notas, los momentos marcados, las respuestas fijadas y la transcripción. Si la guardás de nuevo, se actualiza la misma nota, salvo que la hayas editado. **Exportar** guarda la nota y crea junto a ella un PDF o un Word. **Nueva grabación** descarta la reunión y vuelve al inicio.

Las respuestas en vivo, Notas IA, **Pasar por IA** y las preguntas envían la transcripción al proveedor de IA configurado; las respuestas en vivo y Notas IA también envían los fragmentos de las notas elegidas en **Contexto para la IA** que tienen que ver con lo que se está hablando. La reunión queda en memoria hasta que la guardás, empezás otra o cerrás Notia. Si pasás a otro módulo (por ejemplo, Rutinas) mientras grabás, la grabación y la transcripción siguen; al volver a Meeting ves la reunión como la dejaste, con su tiempo y lo que se transcribió mientras tanto. Si la grabación se cortó sola mientras tanto, Meeting la muestra finalizada con todo lo transcripto, lista para guardar. Si al volver la vista no logra retomar la grabación, **Finalizar** la cierra igual con todo lo transcripto, y **Cancelar** la descarta. Lo mismo pasa mientras separa hablantes. El chat lateral usa la transcripción actual como contexto y el mismo runtime agente, prompt, native tool calling y opciones que el resto de los chats; es efímero, no carga ni persiste memoria global y funciona en modo solo lectura para la biblioteca. Si querés modificar una nota desde ese contexto, abrí un chat persistente. En Windows mezcla el micrófono predeterminado con la salida de audio predeterminada mediante captura WASAPI nativa; no requiere dispositivos virtuales ni cambiar la entrada del sistema.

**Hablantes de Microsoft Teams (extensión NotIA).** Si la reunión es una llamada de Teams abierta en Chrome, la extensión NotIA (carpeta `chrome-ext/notia`) marca en la grabación quién habló en cada momento, con su nombre.
- **Requisitos**: Notia abierto en esta computadora en **modo Host**, y la reunión en Teams para la web (`teams.microsoft.com`) en Chrome. La aplicación de escritorio de Teams no se puede leer desde una extensión.
- **Instalar**: en `chrome://extensions`, con **Modo de desarrollador** activado, tocá **Cargar descomprimida** y elegí `chrome-ext/notia`. En su ícono ingresá el puerto del host (52480 por defecto), el usuario Owner y la contraseña. Queda conectada mientras Chrome esté abierto.
- **Usar**: empezá a grabar en Meeting y entrá a la llamada. Para tener los nombres exactos activá los subtítulos en directo de Teams (**Más → Idioma y voz → Activar subtítulos en directo**). Sin subtítulos, NotIA intenta leer el contorno que Teams dibuja alrededor de quien habla, que es menos confiable. El popup muestra si Notia está grabando, si hay una llamada y quién habla.
- **Resultado**: mientras grabás, cada frase de la transcripción en vivo muestra el nombre de quien hablaba. Al terminar, si la separación de hablantes encontró voces, cada «Hablante N» toma el nombre de la persona de Teams que habló durante la mayor parte de su tiempo, si cubre al menos el 40 %. Si no separó voces, la transcripción se arma por los turnos de Teams. Los nombres quedan en la nota guardada y podés cambiarlos como siempre.
- Solo se marca mientras Meeting graba. Lo que se habló antes de empezar la grabación no se tiene en cuenta.

El acceso **Finanzas**, ubicado debajo de **Transcribir meeting** en el panel izquierdo, abre el módulo local de finanzas personales: un seguimiento informal de los gastos del mes, lo pagado de tarjetas, lo que está en tarjeta a pagar, lo ahorrado, los servicios y los productos con su precio y comercio. La pantalla solo muestra; para cargar o corregir algo, tocá **Cargar con el asistente** o escribile en el chat o por Telegram. Las cuentas solo identifican de dónde salió el dinero y no llevan saldo; las reservas de ahorro sí conservan un saldo a partir de su importe inicial y sus movimientos. Comprar o vender dólares del ahorro es un cambio de moneda: no suma a los gastos ni a los ingresos del mes. Cada biblioteca comienza con diez categorías de gasto y los cargos, intereses e impuestos de las tarjetas van a «Cargos de tarjeta». Los importes usan centavos exactos y ARS/USD se muestran siempre por separado.

La pantalla tiene cuatro pestañas y las flechas junto al título cambian el mes:

- **Resumen**: **Para revisar** (las dudas del asistente; cada botón pone tu respuesta en el chat y nada cambia hasta que la envíes y confirmes), **¿A dónde fue el sueldo?** (el sueldo del mes anterior repartido en tarjetas pagadas, ahorro, servicios pagados fuera de la tarjeta y lo sin registrar), gastos del mes, ahorrado este mes, **En qué se gastó** por categoría con los gastos más grandes, **Tarjetas pagadas** (avisa si las líneas cargadas de un resumen no suman su total), los servicios del mes con su historial y los últimos movimientos. Arriba a la derecha se ven las cotizaciones del dólar.
- **Movimientos**: los movimientos del mes agrupados por tarjeta o cuenta, con búsqueda y filtros (sin categoría, cada categoría, en tarjeta a pagar, ahorro, descartados). Al tocar uno se ve su detalle —fecha de compra, mes en que cuenta, categoría, servicio, de dónde se cargó y estado— y **Pedir un cambio en el chat** lo cita en el chat.
- **Sueldo y ahorro**: el sueldo neto de los últimos meses en pesos o dólares, su promedio, la comparación con la inflación, qué parte del sueldo se va en tarjetas, servicios y ahorro mes a mes, las cotizaciones oficial, blue y tarjeta con tu última compra de dólares, y cada reserva de ahorro con sus movimientos del mes.
- **Productos y tickets**: los productos con el precio más barato y el historial por comercio, y los tickets con su forma de pago y si ya se pagaron en un resumen. Sin productos, explica cómo mandar el primer ticket.

En el teléfono (cuando Finanzas mide menos de 641 px de ancho, también en una tableta angosta o con el Explorador abierto) la pantalla cambia a su versión celular: el botón de chat queda arriba a la derecha y las pestañas se desplazan de costado. **Para revisar** muestra una duda a la vez con flechas, el resumen suma los servicios y los últimos movimientos, y las cotizaciones quedan al pie. En **Movimientos** tocar un movimiento abre su detalle ahí mismo, debajo de la fila. **Sueldo y ahorro** muestra los últimos seis sueldos y las cotizaciones de a dos. En **Productos**, tocar un producto abre su ficha en lugar de la lista y **Productos** vuelve. La pestaña **Dev** (en el teléfono, **Herramientas de desarrollo** al final) permite inspeccionar las tablas.

Los servicios, sus pagos y sus facturas los carga el asistente; los servicios no generan obligaciones futuras automáticamente. En **Resumen**, **Servicios del mes** muestra cada servicio activo como pagado, pendiente o que no corresponde, y **Ver historial de servicios** muestra su estado en los últimos seis meses.

Finanzas se actualiza sola cuando el asistente guarda un cambio, desde el chat de la aplicación o desde Telegram, sin requerir que cierres y vuelvas a abrir la vista. Si falla una consulta, se muestra un error para reintentar.

Las facturas o boletas de servicios pueden llegar como texto, voz, imagen o PDF desde el agente. Se registran asociadas al servicio, período, importe, moneda y, cuando corresponde, al gasto y comprobante original; registrar una factura no crea otro gasto genérico. Si elegís extraer visualmente un documento `service_invoice` con LlamaCloud, usá la acción explícita de extracción y configurá `LLAMA_CLOUD_API_KEY` en el entorno nativo.

Desde **Configuraciones → Finanzas** podés eliminar todos los datos financieros de la biblioteca activa. Tocá **Eliminar datos…** y escribí el nombre de la biblioteca para confirmar; recién entonces Notia borra cuentas, categorías personalizadas, movimientos, tickets, productos y precios, sueldos, ahorro, cuotas, servicios, ocurrencias, facturas y casos para revisar; al terminar vuelve a crear las diez categorías de gasto iniciales. La acción no elimina la biblioteca ni otros datos de Notia y no se puede deshacer.

Los tickets, recibos y resúmenes de tarjeta mantienen el documento original y sus campos normalizados. La extracción visual opcional usa LlamaCloud desde el backend nativo: configurá `LLAMA_CLOUD_API_KEY` en el entorno donde se inicia Notia. El secreto nunca se envía al WebView, pero el documento sí se transmite a LlamaCloud cuando pulsás **Extraer con LlamaCloud**. Sin esa acción, la carga y corrección permanecen locales.

1. Ejecute `bash scripts/install-speech.sh` para instalar Whisper large-v3-turbo, Whisper large-v3 (la segunda pasada de Meeting; solo lo empaqueta el EXE de Windows) y Silero VAD, y compile el runtime de whisper.cpp con `git submodule update --init src-tauri/vendor/whisper.cpp` y `scripts/build-whisper-runtime.ps1 -Platform windows` (Vulkan SDK) o `-Platform android` (NDK) para el APK; el build falla si faltan. La diarización conserva sus modelos ONNX independientes y el runtime documentado en `src-tauri/resources/speech/runtime/README.md`.
2. Abra un chat y pulse **Dictar mensaje sin conexión**.
3. En Android, conceda el permiso de micrófono cuando lo solicite el sistema.
4. Use los controles visibles para pausar, reanudar, finalizar o cancelar. Cancelar restaura el borrador anterior.

En Meeting, Notia mantiene la captura y la transcripción en vivo durante toda la reunión. El audio se conserva temporalmente con memoria acotada y la diarización se ejecuta recién al finalizar, por lo que una reunión de varias horas no se corta cada 15 minutos. El audio temporal se elimina al terminar o cancelar la sesión.

El texto en vivo se actualiza aproximadamente cada medio segundo mientras hablás y cada frase se confirma cuando hay una pausa de medio segundo, o a los 20 segundos de habla continua. Si el equipo está muy exigido, Notia deja de actualizar el texto en curso hasta ponerse al día, para no perder audio; las frases confirmadas siguen apareciendo. Cada frase se transcribe con un volumen parejo, así que una voz baja (un micrófono lejano o la llamada con poco volumen) ya no se pierde, y las respuestas cortas («Sí.», «Claro.») se conservan aunque las diga otra persona. Con el idioma en español, una frase con palabras en inglés se transcribe en español: si el modelo la devuelve entera en inglés, Notia la vuelve a transcribir por partes. Lo que se dice realmente en inglés se conserva en inglés. En Meeting, el archivo temporal admite hasta 12 horas; al llegar al límite, la grabación termina sola y se procesa como si la hubieras detenido. La diarización final se procesa en ventanas de unos 15 minutos, cortadas en una pausa para no partir palabras, para mantener acotada la memoria. Al separar hablantes, Notia reutiliza el texto transcripto en vivo y solo vuelve a transcribir las frases en las que cambia quien habla, así que la espera al detener es mucho más corta. Las etiquetas de hablante se calculan únicamente al finalizar, con la cantidad elegida (cada ventana de 15 minutos se separa en esa cantidad) o, por defecto, automática: Notia corta el audio donde cambia la voz, compara la voz de cada tramo y junta los parecidos, y una voz que aparece muy poco se suma a la más cercana en lugar de quedar como hablante aparte; entre ventanas, cada voz conserva su número; voces solapadas, ruido y fragmentos breves pueden reducir la precisión. Cuando dos personas hablan a la vez, lo dicho en ese tramo queda en el turno de quien empezó primero, sin repetirse en el otro. Si no hay evidencia suficiente para hacer matching, se conserva una etiqueta separada para no fusionar hablantes incorrectamente. Si falla únicamente la diarización, se conserva el texto sin etiquetas. En Android, la primera vez que prepara la voz Notia copia los modelos (unos 700 MB) desde la app al almacenamiento interno, así que tarda más; después ya no. Si Android denegó el permiso permanentemente, habilítelo desde Ajustes. En Windows, compruebe el dispositivo predeterminado y los permisos de privacidad.

![Versión](https://img.shields.io/badge/version-1.0.13-blue)
![Tauri](https://img.shields.io/badge/Tauri-2-orange)
![React](https://img.shields.io/badge/React-19-blue)
![Rust](https://img.shields.io/badge/Rust-2021-orange)

**Notia** es una aplicación de gestión de conocimiento **local-first**, **offline-first** y **privacy-first** para escritorio y dispositivos móviles. Permite organizar notas, documentos, diagramas, credenciales cifradas y tareas, todo viviendo en el filesystem del usuario. Sin servidor cloud. Sin suscripciones. Tus datos, siempre bajo tu control.

---

## 📋 Índice

- [Filosofía](#filosofía)
- [Características Principales](#características-principales)
- [Módulos Funcionales](#módulos-funcionales)
- [Consumo de Funcionalidades](#consumo-de-funcionalidades)
- [Guía de Uso](#guía-de-uso)
- [Requisitos de Sistema](#requisitos-de-sistema)
- [Instalación y Desarrollo](#instalación-y-desarrollo)
- [Configuración de Entorno y Preferencias](#configuración-de-entorno-y-preferencias)
- [FAQ y Troubleshooting](#faq-y-troubleshooting)
- [Licencia](#licencia)

---

## 🧭 Filosofía

- **Local-first**: todos tus datos viven en carpetas locales de tu dispositivo. No hay servidor externo que almacene tu información.
- **Offline-first**: la aplicación funciona completamente sin conexión a internet. La integración con IA es opcional y requiere un servicio local (Ollama).
- **Privacy-first**: tus credenciales en ColdPass se cifran con estándares robustos (AES-256-GCM) y nunca salen de tu dispositivo en texto plano.
- **Cross-platform**: disponible para Windows, macOS, Linux y Android. Además, el mismo programa puede correr como servidor sin ventana en Windows o Linux, para usar Notia desde el navegador de otros equipos de tu red.

---

## ✨ Características Principales

- **Librerías de Notas**: organiza carpetas locales del filesystem como bibliotecas de documentos.
- **Editor de Markdown enriquecido**: edición WYSIWYG con soporte para wikilinks (`[[nota]]`), frontmatter y propiedades. Optimizado para baja memoria con memoización selectiva y selectores de acciones.
- **Enlaces secuenciales (Page Links)**: definí relaciones de orden entre notas con `nextPage` y `previousPage` en el frontmatter. Notia actualiza automáticamente el vínculo inverso y ordena las notas conectadas en bloques secuenciales en el explorador.
- **InkMath en Markdown**: dibujá una fórmula desde los bloques Math y obtené su transcripción LaTeX mediante el modelo de visión configurado en Ollama.
- **Diagramas Mermaid**: incrusta y edita diagramas de flujo, arquitectura y más dentro de tus notas. Los diagramas embebidos en Markdown y los archivos `.mmd` comparten el **mismo motor visual**, temas y estilos. Renderizado lazy con `IntersectionObserver`, cancelación vía `AbortSignal` y caché LRU.
- **Graph View**: visualización interactiva de relaciones entre notas mediante nodos y conexiones. Las notas de Gimnasio (`Gym/`) y de Recetas (`recipes/`) no aparecen en el grafo ni en su buscador, porque son datos de esos módulos; el buscador del Explorador las sigue encontrando.
- **Contextos**: **Configuraciones → Contextos** lista cada tag con su color y permite crear nuevos desde la fila superior, renombrarlos, cambiar su color o eliminarlos, salvo el último o uno que use un tablero (`#Laboral`, `#Personal`, `#Academico`, `#Confidencial` rojo y `#Publico` ámbar por defecto). `#Confidencial` y `#Publico` son predeterminados: están en toda biblioteca, se les puede cambiar el color pero no el nombre, y no se pueden eliminar (si faltan, Notia los vuelve a agregar). Además existe `#System`, un contexto reservado de Notia para los archivos que escribe solo la IA: no aparece en esta lista, no se puede crear, elegir para una nota o carpeta ni asignar a un usuario. Las notas nuevas empiezan con `contexto: "#Personal"`; Graph View colorea los tickets según el contexto aplicado a su tablero y muestra un chip por cada contexto con notas.
- **Clima**: en **Configuraciones → Clima** elegís el lugar del pronóstico de la biblioteca: buscá la ciudad (con provincia o país si hay varias con el mismo nombre) y tocá el resultado. Hasta que elijas uno se usa Buenos Aires. El Inicio se actualiza al momento y el asistente lo usa cuando le preguntás por el clima sin nombrar otro lugar.
- **Cuentas asociadas**: en **Configuraciones → Cuentas asociadas** conectás una o varias cuentas de Gmail a la biblioteca activa (hasta 10) y marcás cada una como laboral, personal o estudiantil. Primero cargá las credenciales de tu propio proyecto de Google Cloud (la pantalla explica los pasos): pegá el **Client ID** y el **Client secret** de un cliente OAuth de tipo App de escritorio o tocá **Importar JSON** para leer el archivo que descarga Google; **Probar conexión** verifica que Google los reconozca y **Guardar** los deja en la biblioteca. Después, **Conectar** abre el navegador para iniciar sesión en Google y autorizar a Notia a leer, enviar, eliminar y mover tus correos; tu contraseña nunca pasa por Notia. La cuenta conectada muestra su **Tipo de cuenta** (Laboral, Personal o Estudiantil), **Reconectar** (por ejemplo, cuando el permiso venció) y **Desconectar**. Las credenciales y los tokens se guardan en `.notia/notiaConfig.json` de la biblioteca, cifrados con la contraseña del Owner, así que viajan con ella si la copiás o la sincronizás y solo se leen después de iniciar sesión.
- **El asistente sigue trabajando**: en el chat principal y en Telegram el asistente encadena todos los pasos que necesita (buscar, contar, borrar, cargar cada ticket…) sin límite de pasos ni de preguntas y sin quedarse en «voy a ver…»: un plan con muchos cambios pide cada confirmación hasta terminar. Lo que escribe entre un paso y otro, las preguntas que te hace, el plan que te propone y tus respuestas quedan en el chat. Cuando decidís un plan (aprobar o **Sugerir cambios**), el plan y tu respuesta quedan como mensajes y la tarjeta se cierra; si sugerís cambios, el asistente te propone un plan corregido. Si algo falla después de que el asistente empezó a trabajar, tu mensaje y el motivo quedan en el chat (por ejemplo, «No pude terminar: …») para que puedas pedirle que siga; si falla antes de hacer nada, el mensaje vuelve al cuadro para reenviarlo. Si una herramienta falla después de que confirmaste, el asistente recibe el error y lo corrige en lugar de cortar. Mientras trabaja, Telegram muestra en el mensaje de progreso lo que está haciendo; cuando llega la respuesta, ese mensaje desaparece. Al tocar una opción de una confirmación, un plan o una pregunta, ese mensaje pierde los botones, muestra lo que elegiste y sigue mostrando el progreso debajo, sin sumar mensajes. La misma consulta corta que elige las áreas decide si el pedido necesita pensar: una charla se responde enseguida, sin thinking, y un pedido con trabajo usa el nivel configurado. En Telegram también elige cómo responder: si contás algo de tu vida, compartís una alegría o una preocupación, te desahogás o solo querés charlar, el asistente responde como un amigo cercano (se alegra o te acompaña, te pregunta cómo fue y no te ofrece cargar nada); si pedís algo concreto, aunque lo cuentes con emoción, responde como asistente.
- **Adjuntos sin texto**: en el chat de la app y en Telegram podés mandar solo una foto, una imagen o un PDF, sin escribir nada. El asistente mira qué es y hace lo que corresponde: carga un ticket o un resumen en Finanzas, copia las reuniones de una captura de calendario o guarda unos apuntes como nota, y si no queda claro, te pregunta.
- **Inicio de sesión**: al abrir Notia aparece la ventana de inicio de sesión de la biblioteca: solo el usuario **Owner** entra a la app. Si el Owner todavía no tiene contraseña, activá **Primer inicio**, escribí su usuario (por defecto «Owner»), tocá **Continuar** y creá la contraseña (al menos 8 caracteres). Desde ahí `.notia/notiaConfig.json` (tokens, API keys, cuentas de Gmail, contextos) queda cifrado con esa contraseña: **si la olvidás, esa configuración no se puede recuperar** y hay que volver a cargar tokens y cuentas. **Recordar sesión** mantiene la biblioteca abierta en este equipo (el bot de Telegram, la sincronización y el agente autónomo arrancan solos al prender el equipo) y **Recordar datos** completa usuario y contraseña; en Windows quedan protegidos con tu cuenta de Windows y en Android en el almacenamiento privado de la app. **Cambiar contraseña** pide la actual y una nueva. Para cerrar la sesión, **Configuraciones → Usuarios → Cerrar sesión**. Mientras la biblioteca está bloqueada no corren el bot, la sincronización ni el agente autónomo.
- **Confirmar todos o proponer otra cosa**: cuando el asistente te pide confirmar un cambio o un plan, en el chat y en Telegram elegís entre **Confirmar**, **Confirmar todos**, **Proponer otra cosa** y **Cancelar**. **Confirmar todos** aprueba ese cambio y todos los que siga pidiendo en ese mismo pedido, sin volver a preguntarte (útil para tareas largas, como ordenar todo el correo); el próximo pedido vuelve a preguntar. **Proponer otra cosa** no aplica el cambio: escribís qué querés en su lugar y el asistente sigue con eso. En Telegram, después de cada respuesta el progreso sigue en un mensaje nuevo al final del chat.
- **Mensajes mientras trabaja**: podés escribir mientras el asistente responde. Si pedís frenar («cancelá», «pará»), corta lo que está haciendo; si pedís otra cosa, queda en cola para después, y si pedís frenar y hacer otra cosa, corta y sigue con lo nuevo. En la app los mensajes en cola se ven sobre el compositor y se pueden quitar; `/cancelar` en Telegram corta siempre.
- **Telegram espera a que termines de escribir**: si escribís tu pedido en varios mensajes, Notia espera 10 segundos desde el último antes de responder, y después lee todo lo que mandaste —textos, audios, fotos y archivos— como un solo pedido. Cada mensaje nuevo vuelve a empezar la espera. Un texto de más de 4096 caracteres que Telegram parte en varios mensajes también llega entero. No espera cuando contestás una pregunta o una confirmación del asistente, cuando le escribís mientras está trabajando en un pedido (ahí decide si lo cancela o lo deja en cola) ni con los comandos como `/cancelar`; `/cancelar` mientras espera descarta lo que estabas escribiendo. Si Notia se cierra durante la espera, al volver te avisa que ese pedido quedó interrumpido y hay que mandarlo de nuevo.
- **El asistente elige sus herramientas**: en el chat principal y en Telegram, antes de responder el asistente decide qué áreas necesita tu pedido (notas, tareas, finanzas, rutina o correo y calendario) y trabaja solo con esas, así responde mejor y más rápido. Puede combinar áreas, por ejemplo «mandale por mail a Juan el resumen de gastos». Solo elige entre las áreas que tu usuario tiene permitidas.
- **La reunión con la IA**: con una reunión abierta en Meeting, el chat de IA de la derecha (y también el chat principal o Telegram) puede trabajar sobre ella: pedile que **modifique o agregue Notas IA** («agregá la decisión de que el corte es a las 18»), que **cambie el contexto** que usa la IA de la reunión (toda la biblioteca, una carpeta o ninguno; queda guardado para las próximas), o que **corrija la transcripción** («la palabra "carreo" en realidad es "correo"»): reemplaza la palabra completa en toda la reunión, sin distinguir mayúsculas, y te dice cuántas veces la cambió. Los cambios piden tu confirmación y solo funcionan para el Owner, en el equipo donde está la reunión. La confirmación de una corrección lista cada reemplazo con cuántas apariciones cambia, antes de tocar la reunión (antes el chat cortaba con «El runtime backend no devolvió una respuesta final»).
- **Correo y calendario con la IA**: con las cuentas de Gmail conectadas, podés pedirle al asistente (chat, chat de una nota, Task Manager o Telegram) que busque y lea tus correos, los mande a la papelera, los mueva de carpeta, los marque como spam o como leídos, envíe o responda correos, y que lea o cree eventos en tu Google Calendar. Antes de cambiar o enviar algo te muestra exactamente qué va a hacer y espera tu confirmación. El asistente sabe de qué cuenta es cada correo o evento y de qué tipo (no es lo mismo un correo de la facultad que del trabajo): las búsquedas recorren todas las cuentas y te dice de cuál es cada resultado, y para enviar o crear un evento usa la cuenta que le digas (por dirección o por tipo, como «desde la laboral») o te pregunta. No sigue instrucciones que vengan dentro de un correo. Es información confidencial: lo usan el Owner y quienes tengan acceso a #Confidencial. Si conectaste la cuenta antes de que existiera el calendario, reconectala para darle ese permiso.
- **El asistente te escribe solo**: con el bot de Telegram activo y tu usuario Owner vinculado, cuando llega un mail nuevo a Recibidos de alguna cuenta de Gmail conectada el asistente lo mira y, si vale la pena, te escribe por Telegram (solo a vos, el Owner): un recordatorio, una pregunta o una propuesta para organizarte. Si no hay nada para decirte, no manda nada. En esas revisiones solo lee: si propone un cambio, respondele «dale» y lo hace en el chat con la confirmación de siempre. Anota lo que ya te dijo para no repetírtelo y respeta tus reglas, por ejemplo «no me mandes mensajes de noche». Si le escribís mientras revisa, tu mensaje va primero. Se apaga con el interruptor **Agente autónomo** de **Configuraciones → Telegram**. La revisión de cada hora de tu agenda, tareas, rutina, correo y finanzas es la acción **Revisión de cada hora** de **Acciones IA**.
- **Usuarios y permisos de contexto**: en **Configuraciones → Usuarios** cada usuario puede tener contextos permitidos, que se activan o desactivan con un toque sobre cada uno. Owner tiene todos los contextos por defecto y esa condición queda protegida.
- **AI Chat local**: conversación con modelos de lenguaje ejecutados localmente vía Ollama, con un motor común para el chat principal, chats desplegables, Meeting, Telegram y la publicación. Cada solicitud usa el `libraryUserId` estable de la biblioteca y autorización por contextos; Meeting y la publicación no persisten memoria global. Telegram usa memoria persistente únicamente cuando el vínculo autorizado corresponde al Owner (`libraryUserId: user-owner`); para cualquier otro usuario conserva la política `ephemeral-no-memory`. Finanzas exige `#Confidencial`. Las respuestas nativas vacías transitorias después de una ronda de tools se reintentan de forma segura.
- **Inicio**: el primer acceso de la barra izquierda (el ícono de la casa) abre un tablero con lo más importante de la biblioteca: la agenda de los próximos 7 días, tus tareas más urgentes con el Pomodoro, el mes de Finanzas con el dólar, los hábitos de hoy, el anotador del día, los chats, notas y reunión para retomar y el clima de tu ciudad. Desde ahí podés preguntarle al asistente, crear una nota o empezar a grabar una reunión.
- **Agenda**: calendario mensual, semana en bloques de 15 minutos para agendar tareas con prioridad, anotador rápido del día y próximos eventos, guardados en la biblioteca para cada usuario.
- **Agentes en el Chat IA**: cada chat puede sumar hasta seis agentes de `.agent/promps/` y una dinámica de `.agent/dynamics/`; los agentes responden en rondas y entre ellos, con los permisos, la memoria y el contexto permanente del chat. Reemplaza a Multichat.
- **Ediciones del agente a la vista**: cuando el chat (el principal o el desplegable de la derecha) edita, crea, borra o deshace una nota que tenés abierta, el editor muestra el cambio en el momento, también al tocar **Confirmar** en una edición propuesta; no hace falta cerrar y volver a abrir la nota.
- **ColdPass**: gestor de credenciales cifradas con generador de contraseñas y sincronización segura entre dispositivos vía Bluetooth.
- **Task Manager**: tableros con vistas Tablero, Lista y Management (plan con ruta crítica), personas y proyectos por tablero, jornada, sprints y divisiones del año, cronómetro por tarea, deshacer y temporizador Pomodoro integrado.
- **Task Manager en el teléfono**: cuando la vista es angosta (un teléfono, o una ventana de menos de 600 px) el Task Manager cambia a su versión para el dedo, igual en la app y en el tablero publicado. Abajo hay cuatro secciones: **Tareas**, **Pomodoro**, **Archivo** (con la cantidad de completadas) y **Ajustes** (editar el tablero; en el tablero publicado no aparece). En **Tablero** ves una columna por vez: tocá un chip de arriba, las flechas o deslizá de costado para cambiar; cada tarjeta muestra prioridad (tocala para cambiarla), el tiempo corriendo, el anillo de horas y los botones **Iniciar**, **Pausar** y **Finalizar**; el **⋯** abre una hoja con prioridad, **Mover a**, duplicar, desestimar y eliminar. **Lista** agrupa por columna con filtros rápidos (Todas, Mías, En progreso, Urgentes y altas) y un círculo para finalizar. Tocar el título abre el **detalle**: título, prioridad, proyecto, descripción, tiempo con botones para sumar o restar y estimaciones rápidas, personas, subtareas y comentarios; desde su **⋯** se abre la nota completa en el editor. **Management** muestra la división, el rango, los indicadores, el Gantt (se desliza de costado), la ruta crítica paso a paso y las tareas del rango con su cálculo. El botón **+** crea una tarea en la columna que estás viendo.
- Cada ticket puede tener un **proyecto** (por ejemplo, Remesas o Corresponsalías) y varias **personas asignadas**; se eligen al crear la tarea, se ven en la tarjeta y se cambian en el editor del ticket (propiedades `proyecto` y `asignados`). Arriba del tablero y de las listas hay una **búsqueda** (título, detalle, proyecto o persona) y filtros por **estado, prioridad, proyecto y persona**; una subtarea que coincide se muestra con su tarea. El detalle de la tarjeta se ve con su formato Markdown. **Finalizar** y **Desestimar** piden confirmación, y cada acción importante muestra un aviso arriba a la izquierda, como «Tarea finalizada». Al cambiar el estado, la prioridad, las horas, una subtarea o un comentario, la tarjeta queda donde estaba; al arrastrar un ticket con el mouse cerca del borde de arriba o de abajo, el tablero se desplaza. El lápiz de la tarjeta abre el mismo editor que el doble clic, y el comentario se escribe con el editor de las notas. Todo esto vale igual en el tablero publicado.
- Task Manager trabaja sobre la biblioteca abierta: crear, editar, mover y ordenar tareas, administrar tableros y grupos y registrar los ciclos Pomodoro se resuelve y valida en el backend de Notia, igual en Windows y Android. Sin una biblioteca abierta, el tablero lo indica y no ofrece elegir otra carpeta. El temporizador Pomodoro lo lleva el backend para cada biblioteca y usuario: sigue corriendo si cerrás y volvés a abrir Notia, y cada persona de un tablero publicado tiene el suyo.
- Cada tablero de Task Manager tiene un contexto seleccionable al crearlo o editarlo; todos sus archivos `.md` se sincronizan con la propiedad `contexto` del tablero.
<!-- La descripción histórica siguiente se conserva fuera de la vista para evitar reintroducir el flujo retirado de contraseña de tablero y aprobación de dispositivos.
- **Publicación de Task Manager en red local (Windows)**: desde **Configuraciones → Publicar** podés seleccionar tableros, configurar una contraseña y abrir el mismo `TaskManagerApp` en un navegador de la red local mediante HTTPS y la ruta estable `/task-manager`, con las vistas Kanban/tabla y las mismas operaciones de tareas, incluida la creación de tarjetas, subtareas, comentarios y movimiento. Las tarjetas nuevas conservan el contexto configurado en su tablero, y el backend de publicación también conserva ese contexto al sanitizar y distribuir los settings. “Nueva tarea” inicia siempre una tarea principal; “Subtarea” es la única acción que precarga un padre. Al crear una subtarea, Notia valida que la tarea padre siga existiendo como tarea principal del mismo tablero antes de crear el archivo; si el estado publicado quedó desactualizado, conserva el tablero y muestra un error para recargar, evitando archivos huérfanos. Los cambios realizados por cualquier navegador autorizado se notifican inmediatamente a los demás navegadores y a la instancia host de Notia. La barra superior publicada permite abrir un chat lateral efímero que usa el mismo runtime y tool calling nativo de la app, mostrando feedback operativo y la respuesta en streaming; puede consultar y modificar únicamente archivos y tickets de los tableros publicados, sin reglas, memorias ni documentos globales de la biblioteca. La página publicada no permite crear, editar ni eliminar tableros; solo expone los elegidos por el anfitrión. En Android, mantené pulsado un ticket y arrastralo con el dedo hasta otra posición o grupo para moverlo; el toque normal conserva las demás acciones visibles. Notia recuerda los tableros, el hash de contraseña y los dispositivos autorizados, y vuelve a publicar automáticamente al iniciar cuando hay una biblioteca activa. Un dispositivo nuevo queda esperando aprobación en esa sección; desde allí se puede aceptar o revocar un acceso ya otorgado, cerrando sus sesiones activas de inmediato. En un navegador autorizado, el checkbox **Recordar contraseña en este dispositivo** conserva la contraseña cifrada para los próximos ingresos en ese mismo navegador. El servidor escucha en la interfaz LAN, exige una sesión autenticada y autoriza el filesystem exclusivamente para los tableros publicados mientras Notia está abierta. Genera y conserva un certificado autofirmado, que cada dispositivo remoto debe aceptar o instalar una vez. La contraseña no se persiste: se conserva únicamente su hash PBKDF2-HMAC-SHA256 con salt.
-->
- **Publicación de Task Manager en red local (Windows)**: desde **Configuraciones → Publicar** seleccioná los tableros que querés exponer y compartí la URL HTTPS estable `/task-manager`. El acceso usa usuario y contraseña de la biblioteca, sin contraseña adicional de tablero ni aprobación de dispositivos. La vista publicada es la misma del Task Manager de la app y permite consultar y modificar únicamente tickets de los tableros publicados; valida contexto, ticket padre, revisión y conflictos y sincroniza cambios por WebSocket. Tocar el título de un ticket lo abre en una pestaña con el mismo editor de la app (nombre, «Guardado», zoom, menú, modo página, lápiz y trazos sobre el ticket), y se guarda solo un momento después de cada cambio; **Tareas**, en el menú de la izquierda, vuelve al tablero. Arriba, la barra muestra el logo y dónde estás (la biblioteca y la vista o la nota abierta), y a la derecha el estado de la conexión («Sincronizado · hace 2 min»; tocalo para sincronizar ahora), «Publicado», **Copiar enlace** (copia la dirección para compartir), **Asistente** (abre el chat), el cambio de tema claro u oscuro (queda guardado en ese navegador) y tu cuenta, con **Cerrar sesión**. El botón del panel de la izquierda muestra u oculta el explorador. A la izquierda hay un menú con tres entradas: **Explorador**, **Tareas** y **Grafo**; en el teléfono el menú y el explorador aparecen juntos al tocar ese botón. **Explorador** abre y cierra el árbol de carpetas y notas (también la flecha doble de su encabezado lo oculta) y **Grafo** muestra la vista de grafo; los dos muestran únicamente las notas (y las carpetas que las contienen) cuyo contexto es el de los tableros publicados, además de los tickets publicados. Desde el explorador se abren las notas en pestañas con el mismo editor, y se pueden crear notas y carpetas, renombrar, mover y eliminar (en el menú contextual o manteniendo pulsado en táctil). Una nota nueva nace con el contexto del tablero que estás mirando, y el contexto de una nota no se puede cambiar desde la publicación: la propiedad se ve con un candado y Notia rechaza el guardado si cambia. Las carpetas con notas de otros contextos no se pueden renombrar, mover ni eliminar, y la carpeta del Task Manager no se edita desde el explorador. En el teléfono el explorador flota sobre el tablero, como en la app. El chat lateral es el mismo panel de la app, efímero, sin elegir agentes ni historial: las preguntas y confirmaciones del agente aparecen en ese chat y el agente solo usa las herramientas de Task Manager sobre los tableros publicados. El invitado no crea, edita ni elimina tableros. Lo que todavía no ofrece la publicación: adjuntar archivos o dictar en el chat, convertir trazos con IA, exportar el ticket, abrir enlaces a otras notas, sugerencias de `[[enlaces]]`, buscar notas por su contenido (el buscador del explorador filtra por nombre), diagramas Mermaid y archivos que no sean notas, el historial de Pomodoro y el contenido de la biblioteca que un bloque GitBook incluye. Los cambios de página y lápiz quedan en la página, no en el host. Las sesiones y streams se cierran al vencer o revocar el acceso. Con **Recordar sesión** marcado al ingresar, el navegador entra directo durante 30 días, aunque cierres y vuelvas a abrir Notia o cambies los tableros publicados; el acceso se corta si el Owner deja de publicar, te quita o cambia la contraseña, o si cerrás sesión.
- **Autenticación vigente de la publicación**: la URL `/task-manager` solicita únicamente el usuario y la contraseña de una cuenta de la biblioteca. No hay contraseña adicional de tablero, aprobación de dispositivos ni credenciales guardadas en el navegador; al vencer o revocarse la sesión, se cierran sus conexiones y streams activos.
- **Búsqueda integrada**: búsqueda de archivos por nombre dentro de la librería activa.
- **Temas**: soporte para tema claro y oscuro.
- **Multiplataforma**: Windows, macOS, Linux y Android.
- **Bandeja del sistema en Windows**: al cerrar la ventana principal, Notia continúa ejecutándose en segundo plano y puede restaurarse desde el icono junto al reloj.
- **Backups en Windows**: configurá una carpeta del sistema para comprimir la biblioteca activa cada hora, conservando hasta 48 copias (2 días).
- **Agente por Telegram**: vinculá de forma explícita un chat privado con un usuario de la biblioteca para buscar, leer y modificar la biblioteca activa desde el bot. Cada escritura, incluida una operación financiera, muestra preview, una única confirmación visible y un resultado verificado antes de informarse como exitosa.

Desde Telegram, el agente tiene acceso transversal a la biblioteca activa: puede buscar reuniones y notas, consultar y modificar (con confirmación) tickets y grupos de cualquier tablero de Task Manager y usar las herramientas de Finanzas en la misma conversación. El módulo seleccionado en la interfaz no limita esas consultas; cada lectura y escritura sigue la autorización del usuario y sus contextos, y Finanzas requiere `#Confidencial`. Los pedidos de sueldos ya cargados se resuelven con los datos locales, sin búsqueda web, y las llamadas de herramienta idénticas no se repiten dentro de la misma operación.

Las respuestas del agente en Telegram no usan Markdown. El modelo utiliza texto plano y, cuando hace falta, un subconjunto básico de HTML compatible con Telegram (por ejemplo, negrita, cursiva o código); el resto de los chats conserva su formato habitual.

Mientras una solicitud de Telegram avanza, Notia mantiene un único mensaje de estado editable que comienza como **Solicitud recibida y en proceso**. Cuando llega la primera señal del thinking, ese mismo mensaje cambia a etapas claras como preparación, lectura, organización de pasos, ejecución y verificación. Si la solicitud es compuesta fuera de Finanzas, muestra el TO-DO con el estado de cada paso; cuando Finanzas está habilitada, Telegram no expone herramientas de planes de ejecución y limita cada turno a una mutación financiera confirmada. Las aclaraciones y confirmaciones aparecen en mensajes separados con sus botones. El estado muestra resúmenes breves del avance, nunca el razonamiento interno completo del modelo ni datos privados.

Las preferencias de feedback permiten elegir modo mínimo, estándar, detallado o desactivado, mostrar/ocultar el TO-DO y decidir si Telegram edita un único mensaje de progreso. La búsqueda web pública no requiere confirmación adicional ni modifica la biblioteca. Las consultas de noticias actuales, incluso si mencionan finanzas, se envían al scope con búsqueda pública; el agente no puede finalizar sin ejecutar la búsqueda y, si encuentra fuentes, debe incluir enlaces devueltos por ella. Las búsquedas web sanitizan la consulta y los resultados como contenido no confiable: no envían contexto privado, claves, tokens ni datos personales, y cada operación admite como máximo seis búsquedas únicas normalizadas, contando errores, cancelaciones y respuestas vacías.

Cada biblioteca mantiene además `.agent/memory/` con `rules.md` (las instrucciones que le diste al asistente), `memory.md` (tus datos y preferencias), `thoughts.md` (los pensamientos del propio asistente: qué notó y qué te avisó, preguntó o propuso, con fecha), `biography.md` (tu biografía, escrita como un libro, con capítulos por etapa de tu vida, a partir de lo que le contás al asistente) `talk.md` (cómo hablás y escribís, para que el asistente te hable parecido) y `people.md` (quién es cada persona que nombrás, una por línea: «Nombre — quién es»). Notia crea automáticamente esa carpeta y esos archivos si no existen, y si borrás `thoughts.md`, `biography.md`, `talk.md` o `people.md` vuelven a aparecer en menos de un minuto. La primera vez, `people.md` se llena con las personas que ya figuran en tu memoria y tu biografía. Cuando nombrás a alguien que el asistente no conoce y es importante para lo que pedís, te pregunta una sola vez quién es y lo anota; si ya lo sabe, lo anota solo. Cada vez que cambia cualquiera de estos archivos, el asistente los revisa todos juntos en una sola pasada: une lo repetido, deja cada dato en un solo archivo (por ejemplo, lo que sabe de otra persona va a `people.md` y no también a la memoria), completa `people.md` con lo que sabe y, si hay datos nuevos de tu vida, reescribe la biografía como un relato literario. De vez en cuando, si la charla da o contás algo personal, el asistente puede hacerte una pregunta breve para completar tu biografía; nunca más de una por conversación, y si no querés contestar no insiste. Una vez por día el asistente revisa los seis archivos: une lo repetido, arregla su formato y acorta el que se haya extendido demasiado, sin perder lo importante. Cuando uno de los archivos se llena, esa misma revisión conjunta lo reescribe más corto sin perder lo importante; si los pensamientos siguen llenos, los más viejos se descartan solos, así una nota nueva del asistente nunca se pierde. Además, al terminar cada pedido tuyo (aunque haya sido largo, haya fallado o lo hayas cancelado), Notia repasa lo que pasó y guarda lo nuevo que aprendió de vos en la memoria y la biografía, cómo le escribiste en tu forma de hablar y lo que quedó hecho o pendiente en sus pensamientos; nunca guarda contraseñas, códigos ni tarjetas. La memoria persistente se carga para el usuario Owner; las reglas operativas se cargan desde la biblioteca según el agente construido. Las superficies efímeras no cargan ni persisten memoria global, excepto Telegram cuando el vínculo autorizado corresponde al Owner; para cualquier otro usuario de la biblioteca Telegram tampoco carga ni persiste memoria.
`rules.md` siempre conserva las reglas mínimas de Notia y permite agregar reglas personalizadas sin perderlas durante la inicialización.
Las instrucciones permanentes dadas al agente se guardan automáticamente en su bloque interno de `rules.md`, sin pedir confirmación adicional.
`.agent/memory/improve.md` es el registro de automejora del asistente: cuando una herramienta le falla o responde mal, le falta una herramienta para lo que pediste o nota algo de Notia que podría funcionar mejor, lo anota solo, sin avisarte, con fecha y tipo (`error`, `herramienta` o `mejora`). Al terminar cada pedido, el repaso de Notia también anota lo que haya que mejorar. Este archivo es para leerlo vos (o quien desarrolle Notia): el asistente no lo lee ni lo usa como contexto. No repite una mejora ya anotada y guarda las 300 más recientes.
Cómo responde el asistente a lo que más se anotó en `improve.md`:

- **Opciones antes que preguntas**: si pedís alternativas o corregís una propuesta («sin barra»), busca de nuevo con esa condición y te responde con las opciones, cada una con por qué sirve y su recomendación. Usa una pregunta con botones solo para un dato que le falta.
- **Búsqueda por palabras** en ejercicios de Gimnasio y en recetas: cada palabra se busca por separado, sin acentos, conectores ni plurales («mancuernas glúteo», «yogur granola»). Si ninguna ficha tiene todas, trae las que tienen alguna, primero las que más tienen.
- **Confirmaciones más cortas**: al cambiar una rutina se ve solo lo que cambia («reemplazar «Hip thrust» (3 × 7 reps con 100 kg) por «Puente de glúteos a una pierna» (3 × 10 reps)»), y las series iguales se juntan. En Telegram, una confirmación que no toca documentos ya no dice «Documentos afectados: 0».
- **Tickets por sección**: puede sumar ítems a una sección del cuerpo, como «Pendientes», o reemplazar solo esa sección, sin reescribir el resto. Los tickets atrasados se buscan por la prioridad «Atrasado», aunque se pidan como estado.
- **Adjuntos de Gmail**: lee el texto de un adjunto PDF, HTML o de texto (una factura, un resumen) para cargar el gasto. No lee imágenes ni PDF escaneados; si el texto de un PDF sale ilegible (algunas facturas usan fuentes con una codificación propia), te lo dice en lugar de leer caracteres sin sentido.
- **Correos con botones y limpieza de la casilla**: al leer un correo ve también sus enlaces y botones («Iniciar sesión», «Ver pedido») con su dirección. Puede aplicar un cambio a todos los correos de una búsqueda (por remitente o asunto, hasta 500) en un solo paso, mover a una carpeta y marcar como leídos a la vez, y después de cada cambio dice en qué carpeta quedó cada correo.
- **Tickets por varias palabras**: la búsqueda de tickets busca cada palabra en el título, el detalle y las etiquetas, sin acentos ni plurales, y encuentra nombres escritos juntos o separados («ApiSearch» y «API Search»); si ninguno tiene todas, trae los que tienen más. También filtra por cómo empieza el título («Seguimiento»). Un comentario o un cambio de ticket aplicado dice que ya quedó guardado.
- **Finanzas**: para corregir un movimiento alcanza con decir qué cambia (por ejemplo solo la descripción). Si falta algún dato, el asistente sabe cuáles son todos juntos y te los pregunta en lugar de inventarlos (también al dar de alta un servicio). Un gasto que mezcla categorías, como un pedido con supermercado y tabaco, se puede cargar una sola vez dividido por categoría.
- **Recordatorios por Telegram**: los mensajes ya no muestran asteriscos sueltos ni `####` cuando el texto trae una negrita que no cierra o un título; las listas con sangría y numeradas con «1)» se ven como listas.
- **Cortes de Ollama**: si el servidor responde con un error propio (por ejemplo «Internal Server Error»), el asistente lo intenta de nuevo dos veces, a los 2 y a los 5 segundos, antes de cortar el turno. Si igual falla, en Telegram te dice si llegó a hacer alguna acción, así sabés si podés repetir el pedido.
- **Leer un registro de Finanzas**: el asistente puede abrir por su número una compra con las líneas del ticket, un sueldo, un resumen de tarjeta, un plan de cuotas, un servicio o un comprobante, no solo un movimiento.
- **Salud**: después de registrar pasos, agua o una comida responde con el resumen del día ya actualizado.
- **Reunión de recién**: desde cualquier chat o Telegram lee las Notas IA y el final de la transcripción de la reunión abierta en Meeting.
- **Acciones IA y revisión horaria**: cada ejecución lee lo que mandaron sus últimas tres, para no repetir el enfoque. Una propuesta sin respuesta no vuelve en el mismo día y al día siguiente se recuerda una sola vez. Dos pensamientos que dicen casi lo mismo quedan en uno.
- **Cancelar por Telegram**: el mensaje de cancelación trae lo que el agente había averiguado hasta ese momento, así «seguí» lo retoma.
- **Terminal y red doméstica**: sabe que la PC host suele estar en tu red y la usa para diagnosticarla (`ipconfig`, `Get-NetNeighbor`, un barrido de ping). También sabe que es Windows PowerShell 5.1, que no tiene `ForEach-Object -Parallel`.

Los archivos de `.agent/memory/` que escribe solo la IA (`rules.md`, `memory.md`, `thoughts.md`, `biography.md`, `talk.md`, `people.md` e `improve.md`) tienen el contexto `#System`: los abrís con el editor para leerlos, con un aviso de solo lectura arriba, pero no podés escribir en ellos, cambiar sus propiedades ni renombrarlos, moverlos, copiarlos o borrarlos (tampoco las carpetas `.agent` y `.agent/memory`). Los cambian la IA con sus herramientas, el repaso de cada pedido y la revisión de sus archivos; **Borrar memoria** en el panel del chat sigue vaciando `memory.md`. Los prompts, las habilidades y las dinámicas de `.agent` siguen siendo tuyos y editables (`#Confidencial`).
Los datos personales, preferencias y contextos duraderos que mencionás en una conversación los guarda el agente en `.agent/memory/memory.md` en ese mismo turno, sin pedir confirmación, y se reutilizan como contexto en los chats persistentes del Owner y en Telegram cuando está vinculado al Owner. Esto aplica al chat IA principal y a los chats laterales de notas y Task Manager: todos usan las mismas `rules.md` y `memory.md` del motor global. Finanzas y Graph View no guardan memorias.
También garantiza la carpeta `.agent/skills/` para las habilidades del agente: cada archivo `.md` que pongas ahí (también dentro de subcarpetas) se suma al contexto del asistente como una habilidad, es decir, instrucciones para un tipo de tarea que sigue cuando tu pedido coincide. Si el archivo empieza con un frontmatter con `name` y `description`, el asistente ve ese nombre y cuándo usarla. Entran hasta 16 habilidades y 100.000 caracteres entre todas; una habilidad nueva agregada desde fuera de Notia aparece cuando la biblioteca se vuelve a indexar (al abrirla o refrescar su árbol).

### Reglas y memoria del agente

Notia garantiza esta estructura de carpetas por biblioteca, sincroniza el visualizador del prompt default y crea únicamente los archivos de memoria faltantes:

```text
.agent/
├── dynamics/
├── promps/
│   ├── default.md (visualizador sincronizado del prompt default)
│   └── (prompts Markdown alternativos opcionales)
├── memory/
│   ├── rules.md
│   └── memory.md
└── skills/
```

`default.md` se crea si falta y se sobrescribe si su contenido difiere del prompt default embebido. Sirve como visualizador sincronizado, no como fuente de ejecución: la opción **default** siempre usa el prompt del sistema de Notia. Los prompts alternativos `.md` se conservan y solo se cargan al seleccionarlos explícitamente.

`rules.md` separa las reglas mínimas administradas por Notia (`NOTIA_DEFAULT_RULES`) de las instrucciones permanentes aprendidas (`NOTIA_IA_RULES`). Si indicás explícitamente “cuando ocurra X, hacé Y” o “a partir de ahora respondé de esta manera”, el agente guarda la instrucción en el segundo bloque sin pedir una confirmación adicional.

`memory.md` conserva hechos duraderos, como nombre, preferencias, empleo o proyectos actuales del Owner. Esos datos no se consideran reglas de comportamiento. El agente los guarda sin confirmación y los incorpora como contexto en conversaciones persistentes del Owner y en Telegram cuando el usuario autorizado es el Owner; Meeting y la publicación no cargan ni persisten memoria global; el chat de Graph View la lee si el chat tiene memoria, pero no la escribe. Telegram usa `persistencePolicy: 'persistent'` para `user-owner` y `persistencePolicy: 'ephemeral-no-memory'` para cualquier otro usuario de la biblioteca. En el segundo caso tampoco puede exponer ni leer rutas bajo `.agent/memory/` mediante tools.

Cada vez que `memory.md` cambia, Notia pide en segundo plano al modelo configurado que ordene las memorias: une duplicados, resuelve contradicciones quedándose con el dato más reciente y agrupa datos del mismo tema, sin inventar ni perder datos. Esa llamada no tiene herramientas ni recibe la memoria como contexto: solo la lista a ordenar. La conversación no la espera, y si mientras tanto se guardó otra memoria, el resultado se descarta y se ordena de nuevo. Si la respuesta del modelo no es una lista válida, el archivo queda como estaba. `rules.md` no se reorganiza. Si la configuración apunta a Ollama Cloud, la lista de memorias se envía a ese servicio. Cada chat nuevo puede crearse sin memoria desde el panel de contexto de la vista Chat IA; los archivos de chat anteriores que tengan `longTermMemory` en su encabezado se siguen abriendo y esa línea deja de escribirse al guardarlos.

En Telegram, las respuestas finales se transforman al subconjunto HTML permitido por Telegram. Encabezados, listas Markdown o HTML (`ul`/`ol`/`li`), negritas, código y enlaces se normalizan aunque el modelo produzca Markdown; el resto de los chats conserva Markdown. Si un modelo emite accidentalmente una llamada de herramienta XML, Notia intenta recuperarla como tool calling nativo en vez de mostrarla como texto.

En Windows, pulsar la **X** oculta Notia en la bandeja del sistema en vez de finalizarla. Para volver, hacé doble clic izquierdo en el icono de Notia o abrí su menú y elegí **Abrir Notia**. Para terminar completamente la aplicación, elegí **Salir** desde ese mismo menú. Este comportamiento no se aplica en Android, macOS ni Linux.

Para conectar Telegram, creá un bot con BotFather, copiá su token y abrí **Configuraciones → Telegram**. Pegá el token, pulsá **Probar conexión**, activá el interruptor **Bot de Telegram** y enviá `/start` al bot. El interruptor **Agente autónomo** (activo por defecto) deja que el asistente te escriba solo cuando llegan mails nuevos; la revisión de cada hora se maneja desde **Acciones IA**. Notia vincula el chat privado con un usuario de la biblioteca; solo coincidir simultáneamente con el `chat_id` y el identificador de usuario vinculados permite consultar esa biblioteca. Las acciones de creación, reemplazo y eliminación, incluidas las financieras, muestran los botones **Confirmar**, **Confirmar todos**, **Proponer otra cosa** y **Cancelar** antes de ejecutarse; en Finanzas se muestra una sola confirmación por mutación. Revocá el vínculo desde la misma pantalla cuando ya no lo necesites.

El token se guarda en `.notia/notiaConfig.json`, cifrado con la contraseña del Owner. Igual conviene no compartir ese archivo; si el token se expone, revocalo inmediatamente desde BotFather. El bot arranca recién cuando el Owner inicia sesión (o solo, si marcaste **Recordar sesión**). El bot funciona únicamente mientras Notia está ejecutándose y requiere conectividad tanto con Telegram como con el backend de IA configurado.

---

## 📦 Módulos Funcionales

### Inicio

El acceso **Inicio** (el ícono de la casa) es el primero de la barra izquierda, separado de los módulos. Abre un tablero que junta los datos de los otros módulos de la biblioteca abierta; cada tarjeta lleva a su módulo con **Abrir**.

- **Encabezado**: el día, un saludo y un resumen (eventos de la semana, hábitos pendientes y tareas urgentes).
- **Clima**: junto al saludo, un chip con la temperatura, el estado del cielo, la máxima y la mínima de hoy y los próximos tres días. Al tocarlo se abre el pronóstico: sensación térmica, humedad y viento, las próximas horas (ahora y cada tres horas, con probabilidad de lluvia y luna de noche) y los próximos 7 días con su rango de temperaturas; se cierra con la X, con Escape o tocando afuera. Los datos son de Open-Meteo, se renuevan cada 15 minutos y son del lugar que elegís en **Configuraciones → Clima** (Buenos Aires hasta que elijas otro). Sin conexión, el chip dice «Clima no disponible · Reintentar». El cuadro **Preguntale al asistente** manda la pregunta al chat lateral, en un chat nuevo, con el agente del chip (tocá el chip para cambiarlo; es el mismo agente del chat lateral). `Ctrl + K` lleva el cursor al cuadro. **Nueva nota** crea una nota en el explorador y **Grabar** empieza a grabar una reunión con el micrófono y, en Windows, el audio de la computadora (`Ctrl + Shift + R`); la grabación sigue en **Transcribir meeting**. Si hay una reunión terminada sin guardar, el botón lleva a Meeting para que decidas qué hacer con ella.
- **Agenda**: los próximos 7 días con un punto en los que tienen eventos; tocá un día para ver solo sus eventos y otra vez para volver a la semana.
- **Tareas**: cuántas tareas hay en **Sprint actual**, **En revisión** y bloqueadas, las más urgentes (tocá una para abrir su ticket) y el Pomodoro de Task Manager. Si no elegiste tarea para el Pomodoro, **Iniciar** trabaja sobre la más urgente.
- **Finanzas**: los gastos del mes, cuántos no tienen categoría, lo ahorrado, la primera reserva y el dólar oficial, blue y tarjeta. **Revisar en el chat** deja en el compositor del chat lateral lo que queda en «Para revisar»: dudas del asistente, gastos sin categoría o resúmenes que no cuadran.
- **Rutinas**: los hábitos de hoy de cada rutina para marcarlos, la semana, el porcentaje del mes y la mejor racha.
- **Anotador rápido**: el mismo anotador del día de la Agenda: agregá, marcá o borrá pendientes.
- **Seguir donde quedaste**: los últimos chats, las notas que abriste en el editor y la reunión actual; tocá uno para retomarlo. Debajo, las carpetas de la biblioteca: tocá una para verla en el explorador. **Ver historial** abre el Chat IA.
- **Actualización**: el tablero se vuelve a leer al volver a la ventana y cuando el asistente u otra pantalla cambian tareas, finanzas o hábitos. Si un módulo no se puede leer, su tarjeta muestra el motivo y las demás siguen funcionando.

### Librerías y Explorador de Archivos

El corazón de Notia son las **librerías**: carpetas locales del filesystem que la app indexa y presenta como un árbol jerárquico interactivo.

Desde el panel izquierdo (Explorador) podés:
- Navegar carpetas y archivos en forma de árbol expandible. El nombre se muestra separado del formato, que se conserva al renombrar.
- Crear carpetas, notas Markdown o diagramas Mermaid.
- Copiar, mover, renombrar y eliminar archivos mediante el **menú contextual** (clic derecho).
- Asignar un **contexto a una carpeta** desde su menú contextual (**Contexto de la carpeta…**, también con el botón de acciones de la fila en pantallas táctiles). El contexto queda guardado y se muestra a la derecha del nombre de la carpeta, como el formato de los archivos. Todas las notas de esa carpeta y de sus subcarpetas toman ese contexto: las que ya estaban, las que se crean ahí y las que se mueven o copian ahí. En esas notas el contexto aparece con candado en **Propiedades** («Lo define la carpeta»). Una subcarpeta con su propio contexto conserva el suyo. Con **Sin contexto**, las notas quedan libres de tener cualquier contexto. Las carpetas ocultas, la raíz de la biblioteca y la carpeta del Task Manager no pueden tener contexto.
- Buscar archivos por título o contenido con **Buscar archivos**, siempre visible arriba del árbol (`Ctrl + O` lo enfoca; `Esc` lo limpia).
- En escritorio, el árbol se actualiza automáticamente cuando detecta cambios externos en el filesystem.
- En Android, podés elegir una carpeta mediante el selector del sistema (SAF) y refrescar manualmente o con intervalo configurable.
  - En Android, al tocar **Agregar nueva libreria**, el modal ignora los `pointerdown` táctiles del fondo y permanece abierto mientras se elige la carpeta, incluso si el WebView reporta el toque sobre el backdrop mientras el dedo sigue sobre el control. La URI `content://` entregada por SAF se conserva como ruta de la librería y al preparar `.notia/notiaConfig.json`, evitando convertirla en una URI inválida. La configuración inicial se crea directamente desde el permiso de la carpeta seleccionada en un solo comando nativo, sin depender de que el listado SAF ya muestre `.notia`; si un intento anterior dejó esa carpeta o el archivo de configuración, Notia los reutiliza sin borrar ni sobrescribir su contenido. Una entrada del mismo nombre pero de tipo incompatible se rechaza. El selector tiene un límite de espera de 60 segundos; si no responde, muestra un error recuperable y podés volver a intentarlo sin agregar una librería incompleta. Si el sistema no puede resolver o guardar la ruta SAF, muestra el error con el mayor detalle disponible y podés reintentar. Una selección válida cierra el modal solo después de configurar y agregar la librería correctamente. La X visible sigue cerrando el modal con touch; el backdrop también lo cierra cuando se usa mouse.

### Editor de Markdown

El editor de Markdown utiliza una experiencia WYSIWYG (lo que ves es lo que obtenés) basada en Milkdown Crepe. Las respuestas del chat también renderizan fórmulas LaTeX con KaTeX, tanto en bloques `$$...$$` como en expresiones inline. Cada fórmula queda encuadrada y el botón con el ícono de ojo permite alternar entre el render y su código LaTeX.

El chat lateral reconoce el bloque o los bloques seleccionados en la nota, incluyendo su tipo y contenido, para responder sobre esa selección. La selección es opcional: también puede leer directamente el archivo Markdown activo, localizar un bloque mencionado por su encabezado, punto o frase, y agregar contenido antes o después o reemplazarlo mediante herramientas con confirmación antes de guardar; el editor abierto recibe el cambio sin cerrar ni volver a abrir la nota. Antes de escribir, Notia comprueba la revisión de la fuente activa más reciente y solo detiene la operación si detecta un cambio real, para no mostrar conflictos por un contexto del chat desactualizado.

Características:
- **Pestañas múltiples**: abrí varios archivos simultáneamente y navegá entre ellos.
- **Guardado automático**: los cambios se guardan automáticamente tras un breve período de inactividad y se persisten de inmediato antes de cerrar, salir o cambiar de biblioteca.
- **Cambios del agente mientras escribís**: si el agente u otro dispositivo cambia la nota abierta, el cambio aparece en el editor y podés seguir escribiendo. Lo que escribiste y todavía no se guardó nunca se pisa: Notia combina las dos versiones. Si los dos cambiaron la misma línea, quedan ambas, una debajo de la otra, y un aviso pide revisarlas. Si una nota no se puede guardar por otro motivo, cerrar la pestaña ofrece **Cerrar sin guardar** en lugar de dejarla trabada.
- **Wikilinks**: escribí `[[Nombre de Nota]]` para crear enlaces bidireccionales entre documentos. Al hacer clic sobre el wikilink, la nota destino se abre en una nueva pestaña; al hacer clic en el resto de la línea, el cursor se ubica ahí para seguir escribiendo o borrar el enlace. Los enlaces se guardan tal cual, `[[Nombre de Nota]]`, sin barras invertidas. En el editor el enlace se lee solo con su texto («Nombre de Nota», o el alias de `[[nota|alias]]`). Al poner el cursor junto al enlace aparecen los corchetes; si borrás el último, queda `[[Nombre de Nota]` y deja de ser un enlace.
- **Propiedades**: arriba de cada nota, el panel **Propiedades** muestra sus metadatos y se pliega con un toque; plegado, resume el contexto y la fecha. Cada valor se edita según su tipo: el contexto con los de la biblioteca, las fechas legibles, casillas, etiquetas como chips y los enlaces a otras notas con un buscador que también permite **Crear nota**. **Agregar propiedad** pide el nombre y el tipo (Texto, Etiquetas, Número, Fecha, Nota o Casilla).
- **Barra de formato**: al pasar el mouse sobre un párrafo, título, ítem de lista o cita aparece encima una barra que formatea el bloque entero; al seleccionar texto (o un bloque con su handle), la barra formatea la selección. Sirve para cambiar el tipo de bloque (párrafo, títulos, listas o cita) y aplicar negrita, cursiva, subrayado, tachado, código, color de texto, resaltado, alineación y enlaces, o quitar el formato. El subrayado, el color, el resaltado y la alineación se guardan como HTML dentro del Markdown (`<u>`, `<span data-color>`, `<mark>`, `<div align>`), que GitHub y Obsidian también muestran, salvo el color.
- **Mover bloques**: al pasar sobre un bloque se resalta y muestra su handle (el ícono de seis puntos); arrastralo para moverlo. Una línea teal marca dónde va a quedar. Para agregar bloques, usá Enter o escribí `/`. Si debajo de un bloque de código, tabla, fórmula, imagen o bloque GitBook no hay una línea donde escribir, tocá el espacio justo debajo del bloque y se agrega una. Tocar el espacio vacío debajo del final de la nota empieza una línea nueva al final.
- **Bloques de GitBook**: el menú `/` suma avisos (información, éxito, advertencia, error), pestañas, desplegables, pasos, columnas, novedades con fecha y etiquetas, código con título, prompts con «Copiar» y «Ejecutar» (lo envía al chat lateral, en un chat nuevo), contenido condicional, tarjetas, URL embebidas, archivos, enlaces a páginas, contenido reutilizable de otra nota y dibujos hechos a mano, más botones, íconos, variables, imágenes del tamaño del texto y anotaciones (notas al pie que se leen al pasar el puntero o al tocarlas). Los elementos en línea también se insertan desde el «+» de la barra de formato. Todo se guarda con la sintaxis de GitBook, así que las notas siguen siendo compatibles con GitBook Git Sync. Las variables salen de la propiedad `vars:` de la nota y de `.gitbook/vars.yaml` de la biblioteca; las condiciones que dependen del lector publicado se marcan como tales. Al exportar a PDF o Word, los bloques se escriben con formato, el contenido reutilizable se inserta y las condiciones se evalúan.

- **Bloques dentro de tablas Markdown**: una celda puede combinar texto normal con bloques de código (incluidos XGraph y Mermaid), imágenes y otros bloques compatibles con el editor.
- **Indicadores de estado**: visualización de "Guardando...", "Guardado ✓" o "Error ✗" en la pestaña activa.
- **Imágenes de chats persistidos**: al abrir un chat Markdown que conserva adjuntos, Notia muestra sobre el editor hasta 24 previews de imágenes raster válidas; también las muestra en la vista de documento grande. SVG, formatos no admitidos y datos Base64 que no superan la validación permitida no se renderizan.
- **Zoom de lectura**: ampliá o reducí el contenido con `Ctrl + rueda del mouse` en Windows, con el gesto de pinza de dos dedos en Android o con el deslizador junto al estado de guardado. El porcentaje visible y el botón **Restablecer** permiten consultar o volver rápidamente al 100%. En un teléfono o una tablet, donde la hoja no entra completa, el 100% es la hoja ajustada al ancho de la pantalla; al agrandarla, se desplaza también hacia los costados.
- **Diagramas Mermaid embebidos**: insertá bloques de código con lenguaje `mermaid` dentro de cualquier nota Markdown. El editor renderiza el diagrama con el **mismo motor visual** que los archivos `.mmd` (temas Notia, zoom/pan interactivo, manejo de errores uniforme). Los diagramas embebidos son de **solo lectura**: se pueden explorar (zoom, paneo, exportar a PNG/SVG) pero no se pueden editar nodos ni flechas desde el editor Markdown. Desde la versión 1.0.13, el renderizado embebido es **lazy** (solo renderiza cuando el diagrama entra en el viewport), cancela renders previos al cambiar de archivo y gestiona la memoria mediante una caché LRU con límite de tamaño.
- **InkMath**: el botón **OCR** de cada bloque Math abre un lienzo compatible con mouse, stylus y touch. Al terminar de escribir, espera el intervalo configurado, rasteriza los trazos y solicita a Ollama la fórmula en LaTeX; una entrada nueva invalida cualquier resultado anterior.
- **Aspecto de los elementos**: títulos, citas, listas (puntos teal y tareas con casilla), tablas, líneas horizontales, notas al pie, bloques de código, fórmulas, diagramas y bloques de GitBook siguen el diseño del editor. Los bloques de código tienen **Números** y **Ajustar** (solo cambian cómo se ven) y **Copiar**; las fórmulas LaTeX, **Escribir a mano**, **Copiar** y **Ocultar código**; los diagramas Mermaid, **Código**, **Dividido** o **Vista** y **Exportar SVG**, que guarda el diagrama junto a la nota (`<nota> - diagrama.svg`). Al posar el puntero sobre un enlace a otra nota aparece una tarjeta con su carpeta, título, el comienzo del texto, cuándo se editó y cuántos enlaces tiene; en pantallas táctiles, tocar el enlace sigue abriendo la nota.
- **Hoja de la nota**: sin modo página, la nota es una sola hoja con el mismo fondo que las páginas, del ancho de un A3, centrada y sin fin hacia abajo: siempre se puede seguir bajando, y al acercarse al final la hoja se alarga una pantalla más. En modo página se agregan hojas en blanco al bajar; las que quedan vacías desaparecen al cerrar la nota, y las que tienen trazos se conservan. Usa los mismos márgenes que las páginas. En pantallas más angostas que la hoja se achica para entrar.
- **Modo página**: las notas se abren en el editor normal. En el menú «⋯» de la nota, **Modo página** divide esa nota en hojas A3 numeradas, como en un procesador de texto. Queda guardado en la nota como la propiedad `pageMode: true`, así que cada nota vuelve a abrirse como la dejaste; desactivarlo quita la propiedad. La orientación, los márgenes y la numeración valen para todas las notas en modo página y se eligen en **Configuración** (`Ctrl+,` o **Tamaño de página** en el mismo menú). Los títulos pasan a la hoja siguiente junto con lo que introducen. En pantallas angostas, las hojas se achican para entrar. El chip «A3 · Vertical» de la cabecera abre la misma configuración. Las exportaciones a PDF y Word usan el mismo tamaño.
- **Seleccionar varios bloques**: con el **Selector**, arrastrá desde un lugar vacío (los márgenes de la hoja, debajo de la nota o fuera de la hoja) y aparece un rectángulo, como en el escritorio de Windows: los bloques que toca quedan seleccionados y resaltados, listos para copiar, cortar, borrar o dar formato. Arrastrar sobre el texto sigue seleccionando texto y un clic debajo de la nota sigue agregando una línea. Con mouse o lápiz; en pantallas táctiles el dedo desplaza la nota y la selección de texto sigue disponible.
- **Escritura a mano**: arriba de la nota está la barra **Herramientas de lápiz**: **Selector** (para escribir texto; con las otras herramientas la nota no se edita ni abre el teclado), **Lazo**, **Lápiz**, **Resaltador** y **Borrador**, seis colores, la punta (fina, media, gruesa o de 1 a 16 px), **Deshacer trazo** y **Rehacer trazo**. Con el lápiz o el resaltador se escribe sobre toda la nota; en modo página, solo sobre las hojas. Los trazos se ven en los dos modos, sobre el mismo texto: lo escrito en una hoja sigue ahí al salir del modo página, y lo escrito sin modo página aparece en las hojas, corrido con los saltos de página. Con **Lazo** (o con el botón lateral del lápiz en «Selección») rodeá trazos para seleccionarlos: se marca un recuadro con la cantidad, el botón de **IA** y **Borrar**; arrastralos para moverlos, y **Delete** o **Borrar** los quita. El botón de **IA** lee lo que escribiste y lo pasa a la nota, a la altura de los trazos, como texto, una fórmula LaTeX (en línea o en bloque), un diagrama Mermaid, un gráfico xGraph, una tabla o código, según lo que hayas escrito; después quita los trazos. **Deshacer trazo** los devuelve, y deshacer en el editor quita lo agregado. Usa el modelo de IA configurado, que tiene que poder ver imágenes, y solo convierte lo escrito con el lápiz, no el resaltador. Mover y borrar se deshacen con **Deshacer trazo**. **Opciones** guarda en el dispositivo la sensibilidad a la presión, el rechazo de palma, dibujar solo con lápiz (el dedo desplaza la página), el suavizado del trazo y qué hace el botón lateral del lápiz (borrador, selección o nada). Los trazos se guardan en la biblioteca, en `.notia/ink/`, y siguen a la nota si se renombra, se mueve, se copia o se borra; la nota Markdown no cambia.
- **Exportación**: el menú «⋯» exporta la nota como PDF o como Word (`.docx`, importable en Google Docs), con formato y sin las propiedades de la nota: títulos, negrita, cursiva, colores, listas, tareas, citas, tablas, código y enlaces. Las fórmulas LaTeX quedan escritas, no como imagen: en el PDF, compuestas como en el editor; en Word, como ecuaciones editables. Los dos usan el tamaño de página, los márgenes y la numeración de la configuración. En el PDF, los diagramas Mermaid y los gráficos XGraph aparecen como imagen, tal como se ven en la nota con el tema claro; en Word queda su código, igual que si el agente o Meeting exportan la nota. Las imágenes aparecen como «[Imagen: …]» y los emojis, como «?». El PDF solo está disponible para una nota en modo página; en las demás la opción aparece deshabilitada con la leyenda «Requiere modo página». El agente tampoco exporta a PDF una nota que no esté en modo página. La exportación de Meeting no depende del modo página. El archivo se crea junto a la nota; en Android, dentro de la librería SAF activa. Si no se puede escribir, se muestra el error.

#### Bloques dentro de tablas Markdown

Abrí una nota Markdown, colocá el cursor dentro de una celda y usá el menú de bloques de Milkdown para insertar el bloque que necesites. La celda puede conservar texto antes o después del bloque; al guardar, Notia mantiene el contenido y ajusta los bloques, gráficos, imágenes y previsualizaciones al ancho disponible de la tabla. Las celdas de encabezado también admiten este comportamiento.

La tabla conserva su selector/handle de Milkdown y los bloques dentro de celdas también muestran el suyo, incluso si están anidados en una cita (`blockquote`): podés seleccionarlos, eliminarlos con la acción normal del editor y arrastrarlos entre posiciones o celdas. Los nodos intermedios de la estructura (`table_header_row`, `table_row`, `table_header` y `table_cell`) no muestran handles propios; Milkdown asciende hasta la tabla cuando corresponde.

La compatibilidad se conserva mediante comentarios internos que no se muestran en la vista renderizada. No los elimines con un editor externo: si una herramienta elimina esos comentarios, Notia ya no podrá restaurar los bloques no inline de la celda al volver a abrir la nota.

### Diagramas Mermaid

Integración nativa de diagramas tipo Mermaid dentro del ecosistema de Notia.

- Los diagramas se guardan como archivos `.mmd` dentro de tu librería, siempre como código Mermaid estándar.
- El editor de `.mmd` trabaja con cinco tipos: **Flujo**, **Secuencia**, **Estados**, **Clases** y **Entidad-relación**. Elegí el tipo arriba; cambiarlo pide confirmación y empieza un diagrama de ejemplo.
- A la izquierda está el **Código**, con las líneas de lo seleccionado resaltadas y el estado de la sintaxis; según el tipo, también **Formas** e **Íconos** o **Elementos**: tocá una ficha para agregarla (o, con un nodo seleccionado, para cambiarle la forma o el ícono).
- En el lienzo, tocá un elemento para seleccionarlo: aparece un menú flotante con lo más usado y, a la derecha, el inspector con todas sus opciones (texto, forma, colores, tipo de línea, flechas, multiplicidades, cardinalidades, miembros, claves…). Para unir dos elementos, elegí **Conectar** y tocá el origen y el destino.
- Abajo: **Grilla**, trazo **A mano**, **Tema** (Munin, Oscuro, Claro, Neutral, Bosque) o **Dirección**, y el zoom. **Exportar** guarda PNG o SVG.
- `Ctrl+Z`/`Ctrl+Y` deshacen y rehacen, `Esc` cancela y `Supr` borra lo seleccionado. En pantallas angostas, los paneles se abren con sus botones sobre el lienzo.
- El **Graph View** usa `react-force-graph-2d` para visualizar el grafo de wikilinks de la librería en un canvas 2D interactivo.

### Graph View

Visualización gráfica de las relaciones entre todas tus notas.

- Cada nota es un **nodo**; cada wikilink es una **conexión**. Los nodos más conectados son más grandes y el color indica la etiqueta.
- Arriba se ven las notas y enlaces, el cambio entre grafo **Global** y **Local** (la nota elegida y sus vecinas) y un chip por etiqueta para ocultarla o mostrarla.
- Tocar un nodo lo selecciona: el panel de la derecha muestra su carpeta, sus enlaces, **Abrir nota**, **Sumar al chat** y sus conexiones. Sin selección, el panel resume la biblioteca: notas, enlaces, huérfanas, notas por etiqueta y las más conectadas.
- Arrastrar un nodo lo deja fijo; un doble toque lo suelta.
- El buscador lista las notas que coinciden, con su carpeta, y las marca en el grafo.
- Abajo: zoom, **Encuadrar**, nombres automáticos o todos, mostrar u ocultar huérfanas y carpetas, y **Fuerzas** para ajustar la separación, la distancia de los enlaces y cuánto se agrupan las carpetas. Estas preferencias se recuerdan.
- Un minimapa muestra todo el grafo y la parte que está en pantalla.
- En pantallas angostas el panel de la nota aparece abajo.
- Los chats laterales de Graph View, Task Manager y archivos comparten el mismo flujo persistente de creación, selección, hidratación y visualización. Cada contexto usa una clave estable y su propio archivo dentro del historial de chats. En Graph View, sin selección consulta la biblioteca mediante RAG local, incluyendo nombres y rutas de carpetas; por ejemplo, preguntar por `chats` recupera los documentos ubicados dentro de esa carpeta. Al seleccionar archivos usa su contenido completo como contexto directo. También puede buscar y leer por título mediante tool calling nativo de Ollama.
- Durante una consulta con herramientas, el panel muestra si está analizando, ejecutando una búsqueda o procesando resultados. Los modelos grandes disponen de un tiempo ampliado para completar las distintas rondas del agente y la operación se puede cancelar desde el compositor.
- Notia mantiene un archivo `linkCache.md` dentro de `.notia/` con el diagrama del grafo, que se regenera automáticamente en segundo plano cuando cambian las notas.

### AI Chat

Chat con inteligencia artificial local via **Ollama**.

La vista **Chat IA** del menú izquierdo se organiza en tres zonas:

- **Historial** (izquierda): botón **Nuevo chat**, que vuelve a la pantalla «¿En qué trabajamos hoy?» (el chat se crea al enviar el primer mensaje), búsqueda por título, la lista de chats de `chat/chats` y la librería activa. Cada chat tiene un botón **⋯** con la opción **Eliminar chat**; en escritorio también funciona el clic derecho. El historial se oculta y se vuelve a mostrar desde su propio botón; en pantallas angostas y teléfonos flota sobre la conversación y se cierra solo al elegir un chat.
- **Conversación** (centro): una barra con el título del chat, el modelo activo (tocándolo abre **Configuraciones → IA**) y el botón del panel de contexto. Tus mensajes aparecen a la derecha; las respuestas de Notia ocupan todo el ancho y se pueden copiar con un botón. El compositor reúne adjuntar, el interruptor **Toda la librería**, dictado y enviar. Con el interruptor encendido, la IA puede buscar en toda la librería (RAG); apagado, solo usa los archivos y carpetas que elijas y no busca otros. El menú **+** ofrece **Seleccionar archivo**, **Buscar archivos de la librería** y **Buscar carpetas de la librería**: una carpeta suma todos sus archivos, subcarpetas incluidas. El interruptor, los archivos y las carpetas quedan guardados en el chat. Mientras llega una respuesta, el botón de enviar pasa a **Detener respuesta**.
- **Contexto** (derecha): el alcance actual con los archivos elegidos (se pueden quitar uno por uno), acciones rápidas que completan el mensaje y el interruptor **Memoria persistente del agente**. Con el interruptor apagado, el próximo chat nuevo no lee `memory.md` ni guarda reglas o memorias (las reglas de `rules.md` se siguen aplicando), y la barra superior muestra **Sin memoria**. La elección queda guardada en el chat: con un chat abierto, el interruptor muestra su valor y no se puede cambiar. **Administrar memoria** permite vaciar `memory.md`. En pantallas medianas y teléfonos flota sobre la conversación.

Al abrir la vista no se abre ningún chat anterior: aparece un chat nuevo con **¿En qué trabajamos hoy?**, el compositor en el centro y tres sugerencias para empezar. El primer mensaje crea el chat; para retomar uno anterior, elegilo en el historial.

El chat principal, los chats desplegables, Meeting, Telegram y la publicación usan el motor común y el contrato global versionado: biblioteca, request, actor estable, canal, snapshot, scope solicitado y política de persistencia. El motor verifica el actor y los contextos antes de ofrecer o ejecutar herramientas; Finanzas requiere `#Confidencial` para lecturas y escrituras, y Graph View guarda sus chats en `chat/chats/` como el resto, con herramientas de solo lectura. Telegram y la URL pública conservan sus adaptadores de transporte, pero no pueden ampliar permisos mediante `scope`, rutas o IDs enviados por el cliente.

- Configurá la URL de tu instancia de Ollama desde **Settings → IA**.
- Funciona con **cualquier modelo de Ollama**, no solo modelos de visión.
- El selector enumera todos los modelos informados por Ollama y marca Thinking, Vision y Tools; para los chats con agente se recomienda elegir uno con **Tools**.
- La búsqueda web opcional usa el endpoint oficial de Ollama Cloud (`https://ollama.com`) y requiere una API Key; una URL local continúa siendo válida para el chat, pero no habilita búsquedas públicas.
- En Task Manager, las consultas temáticas usan RAG; los pedidos exhaustivos como “todos los tickets” activan una lectura completa del corpus y reportan si algún contenido debió truncarse.
- Podés adjuntar archivos y carpetas de la librería como contexto para la conversación. En modo **Directo** la IA recibe el contenido de los archivos (hasta 30.000 caracteres por consulta; los que no entran se nombran como omitidos). En modo **Referencia** recibe nombres y rutas (hasta 50 archivos) y, si la búsqueda en la librería está encendida, puede leerlos. Cada consulta toma hasta 500 archivos entre los elegidos y los de las carpetas.
- En el chat lateral de un archivo abierto, el archivo activo está autorizado como contexto; la IA solicita permiso visible antes de leer cualquier otro archivo.
- Cuando hace falta información actualizada, la IA puede solicitar buscar fuentes públicas mediante Ollama Cloud. La consulta pasa por un filtro que bloquea secretos, datos personales y contenido privado; la API key nunca se incluye en la búsqueda, en la URL, en los mensajes ni en los logs, y la confirmación del usuario no desactiva esa protección.
- La vista principal de chat, los chats desplegables, Meeting, Telegram y la publicación comparten el agente con tool calling nativo, catálogo filtrado por scope, aclaraciones, planes cuando corresponden y confirmaciones individuales. Cuando Telegram tiene Finanzas habilitadas, su catálogo no incluye herramientas de planes de ejecución y cada turno admite como máximo una mutación financiera confirmada. El contexto activo solo limita qué archivos están autorizados inicialmente y qué tablero se considera activo.
- Telegram autoriza inicialmente el corpus legible completo de la biblioteca y no hereda el módulo activo de la interfaz: sus consultas pueden combinar documentos, tickets de Task Manager y datos financieros.
- El compositor admite dictado y adjuntos de audio mediante ASR/STT; Qwen3-TTS permanece disponible para superficies que lo integren, pero los chats no exponen un modo llamada ni leen automáticamente las respuestas. La sección **Configuraciones → Voz** concentra las opciones de transcripción y síntesis disponibles.
- Cuando el agente necesita una aclaración abierta, muestra la pregunta dentro del hilo y pausa la ejecución. La respuesta escrita en el compositor reanuda la misma consulta; también puede cancelarse mientras espera.
- Cada librería puede mantener prompts Markdown alternativos en `.agent/promps/`. La carpeta `.agent` es visible y editable desde el explorador de Notia, aunque las demás carpetas ocultas continúan excluidas. `default.md` se crea o sobrescribe para mostrar exactamente el prompt default embebido, pero no es la fuente de ejecución: la opción **default** siempre usa el prompt del sistema. Los prompts alternativos `.md` se cargan únicamente cuando se seleccionan de forma explícita y nunca se sobrescriben durante esta sincronización. El chat lateral muestra en su encabezado el agente que responde (su nombre es el primer título del archivo y su descripción, el campo `description` o el primer párrafo); al tocarlo se elige otro de la lista, que incluye `default` y cada archivo adicional. Recuerda la elección en este dispositivo y vuelve al prompt del sistema si el alternativo no puede leerse o está vacío.
- El selector **Adjuntar archivo → Seleccionar archivo** permite elegir varios archivos locales a la vez. Notia los conserva juntos en el compositor, muestra un chip por archivo y permite quitar cada uno individualmente antes de enviar la consulta. Cuando hay muchos adjuntos, se muestran dentro de un bloque compacto con desplazamiento propio para mantener visible el campo de mensaje. Al enviar, los adjuntos quedan asociados al mensaje de usuario, se conservan al guardar el historial del chat y se rehidratan al volver a cargarlo; sus nombres aparecen en el hilo y pueden reutilizarse en consultas posteriores dentro del contexto conservado.
- La IA mantiene **memoria persistente** en `.agent/memory/memory.md` para las superficies persistentes: el agente guarda con su herramienta de memoria los hechos, preferencias y datos personales duraderos que mencionás, y los usa para personalizar respuestas futuras. Meeting y las superficies publicadas no cargan ni escriben esa memoria; el chat de Graph View puede leerla, pero no escribirla. Telegram la carga y puede guardarla cuando el vínculo autorizado corresponde al Owner; para cualquier otro usuario usa `persistencePolicy: 'ephemeral-no-memory'`, no carga ni persiste memoria y filtra las rutas bajo `.agent/memory/` de sus documentos y tools.
- Soporte para modelos multimodales: enviá una o varias imágenes (capturas, fotos) para que la IA las analice (requiere modelo con soporte de visión). En el chat lateral de un `.md`, podés adjuntarlas junto con PDF o texto y pedir **"Insertá esto en el documento"**: la IA conserva el orden, transcribe el contenido respetando párrafos, listas y encabezados, convierte las fórmulas en bloques LaTeX `$$...$$`, muestra una vista previa con confirmación y actualiza la nota abierta.
- El botón **Adjuntar archivo** permite seleccionar varios archivos locales —imágenes, PDF y texto— para enviarlos en una misma consulta, además de los archivos de la librería. Los archivos de texto se agregan como bloques de contexto separados; las imágenes y las páginas renderizadas de los PDF se combinan en una colección visual ordenada para Ollama. Notia valida el tipo MIME o la extensión admitida, el tamaño individual y el procesamiento de cada archivo antes de iniciar la consulta: rechaza archivos de texto vacíos o de más de 10 MB (un texto de más de 400.000 caracteres llega con su comienzo y la IA lee el resto por partes o busca dentro de él), archivos individuales de más de 40 MB y PDF que no pueden procesarse. No hay un límite ejecutable de tamaño total del lote ni una cantidad máxima implementada de imágenes, por lo que esas condiciones no provocan por sí solas el rechazo del lote. Al pedir **"Transcribí estos PDF a este archivo .md"**, Notia procesa las páginas en orden, incluidos PDFs escaneados, y puede insertar el resultado en el Markdown activo con texto, listas, tablas y fórmulas en bloques LaTeX `$$...$$`. Los PDF de más de 24 páginas se rechazan antes de enviarse para evitar una transcripción incompleta; se requiere un modelo con soporte de visión cuando el lote incluye imágenes o PDF.
- Podés pedirle a la IA que ordene la biblioteca, por ejemplo «mové las notas de Inbox a Proyectos/2026». Mueve archivos o carpetas enteras a otra carpeta o subcarpeta, crea la carpeta de destino si no existe y pide confirmación antes de cada movimiento. Si la nota está abierta, la pestaña la sigue a su lugar nuevo. No mueve las carpetas internas de Notia (`.notia`, `.agent`) ni los archivos de tareas, recetas, gimnasio o chats, que se ordenan desde su módulo.
- Generación automática de títulos para las sesiones de chat.
- Streaming progresivo de respuestas en escritorio y Android mediante el bridge nativo. En un navegador conectado a un servidor Notia, las respuestas llegan en vivo desde ese servidor; la página publicada de Task Manager usa su propio canal.
- Cancelación de respuestas en curso.
- Soporte en escritorio, Android y navegador (contra un servidor Notia) a través del mismo backend de Notia; la prueba en dispositivo Android sigue siendo un requisito de plataforma.

### Agentes, dinámica y permisos del Chat IA

El panel **Contexto** del Chat IA (botón de panel en la barra superior) tiene, además del alcance, las acciones rápidas y la memoria:

- **Permisos**: **Uso de herramientas** deja que la IA busque en la librería y ejecute acciones; apagado, responde solo con lo que ya tiene el chat. **Permisos de lectura/escritura** decide si puede crear y editar notas y tareas o si queda en **Solo lectura**. La memoria (`memory.md` y `rules.md`) depende de su propio interruptor, que se elige al crear el chat.
- **Contexto permanente**: instrucciones que la IA tiene en cuenta en cada mensaje del chat, por ejemplo «Respondé corto y citá la nota de origen». Se guarda solo, un momento después de que dejás de escribir.
- **Dinámica**: una guía de cómo conversan los agentes (debate, mesa redonda, análisis…). Son los archivos Markdown de `.agent/dynamics/`. Con **Ninguna**, los agentes conversan igual, sin guía.
- **Agentes**: **Agregar agente** abre la lista de los prompts de `.agent/promps/` con búsqueda; se pueden sumar hasta seis y quitarlos con la cruz. Sin agentes responde Notia.

Con agentes, al enviar un mensaje responden ellos en lugar de Notia: en cada ronda habla uno o varios, en un orden al azar, y se responden entre sí durante hasta cuatro rondas sin que tengas que intervenir, salvo que la dinámica pida esperar tu mensaje o nombre a quién le toca. Cada respuesta muestra el nombre y las iniciales del agente, y mientras uno escribe ves su respuesta en vivo. Los agentes usan los mismos permisos, memoria y contexto del chat. El botón de detener corta al agente que está hablando; lo que dijeron los anteriores queda guardado. Todo el chat, con los mensajes de cada agente, se guarda en `chat/chats/` como los demás.

Estos ajustes se guardan con cada chat y podés cambiarlos en cualquier momento, salvo mientras la IA está respondiendo. Para un chat nuevo, lo que elijas antes del primer mensaje se aplica al crearlo.

### Agenda

El acceso **Agenda** aparece debajo de **Calendario** en la barra izquierda y abre tu agenda personal.

- **Calendario del mes**: tocá un día para ver su semana; **Hoy** vuelve al día actual y las flechas cambian de mes. Los días con tareas agendadas tienen un punto.
- **Feriados de Argentina**: los días feriados se pintan con el color de su tipo y muestran su nombre (inamovible en coral, trasladable en oro, puente turístico en violeta y no laborable en gris con borde punteado); los días que solo cierran los bancos aparecen como «Feriado bancario». La leyenda debajo del calendario explica cada color. La tarjeta **Feriados** cuenta los días hasta el próximo feriado (sin contar los no laborables) y muestra el siguiente; al tocarla, la agenda salta a ese día. Los feriados se consultan a argentinadatos.com una sola vez por año (nacionales y bancarios) mientras la app está abierta; al llegar a otro año en el calendario se consulta ese año. Sin conexión, la tarjeta lo avisa y se vuelve a intentar a los dos minutos.
- **Semana**: una grilla de lunes a domingo dividida en bloques de 15 minutos, que arranca en las 8:00. Con mouse, hacé clic o arrastrá sobre los bloques; en pantallas táctiles, tocá cada bloque o mantené presionado y deslizá; con teclado, movete con las flechas y seleccioná con Enter o Espacio. Después escribí el nombre, elegí la prioridad (Urgente, Alta, Media o Baja) y tocá **Agendar**. Si elegís bloques separados o de varios días, se crea una tarea por cada tramo seguido.
- **Tareas agendadas**: tocá una para ver su prioridad y horario, y eliminarla con **Eliminar tarea**. Las tareas pueden superponerse: las que comparten horario se muestran lado a lado, y al tocar una la barra avisa «Se superpone con …». Una franja angosta a la derecha de cada día queda libre para elegir bloques debajo de otra tarea (con teclado, las flechas también pasan por debajo).
- **Google Calendar**: con cuentas de Google conectadas en **Configuraciones → Cuentas asociadas** (con el permiso del calendario), la Agenda se sincroniza en las dos direcciones cada 5 minutos con todas las cuentas, desde 30 días atrás hasta un año adelante. Los eventos de cada cuenta aparecen en la Agenda con prioridad Media; los que creás en Notia van a tu primera cuenta personal (o a la primera cuenta si no hay personal). Si el mismo evento cambia en los dos lados, gana Notia. Borrar un evento de un lado lo borra del otro. Los eventos de día completo no se traen, y los que pasan la medianoche terminan a las 24:00 en la Agenda.
- **Anotador rápido**: una lista de pendientes del día. Las tareas que marcás como hechas quedan tachadas hasta que termina el día; las que siguen pendientes pasan al día siguiente.
- **Próximos eventos**: las próximas 10 tareas que todavía no terminaron; al tocar una, la semana salta a ese día y la muestra.
- **Datos**: se guardan en la base de la biblioteca, separados por usuario.
- **IA**: desde el chat o Telegram podés pedirle al asistente que agende un evento en la Agenda de Notia («agendá Iron Maiden el 21/10 a las 21 en Notia»), que te diga qué tenés, que borre un evento o que anote, marque o borre un pendiente del anotador. Cada cambio pide confirmación. Si pedís agendar en Notia y en Google Calendar, lo crea solo en Notia y la sincronización lo lleva a Google. Un evento no puede pasar la medianoche; para eso agenda dos.

### Rutina

El acceso **Rutina** aparece debajo de **Agenda** en la barra izquierda y abre un panel para convertir tareas en hábitos.

- **Rutinas**: agrupá las tareas en rutinas con su propio checklist (Mañana, Noche, Fin de semana...). Podés crearlas, renombrarlas y eliminarlas cuando están vacías; siempre queda al menos una.
- **Tareas**: cada tarea tiene nombre, rutina, categoría de la rueda de la vida, días en que aplica (todos o algunos) y una nota opcional. Se pueden pausar sin perder el historial, editar, eliminar con **Deshacer** y ordenar arrastrando el asa o con las flechas del teclado.
- **Seguimiento**: cada tarea cuenta desde el día en que la creaste (los días anteriores no son días perdidos, salvo que los marques como hechos). Panel de hábitos con la racha de cada tarea, calendario del mes, evolución diaria comparada con el mes pasado, progreso semanal, rueda de la vida con metas por categoría y la semana actual para marcar lo hecho, con la pestaña **Todos** (cada tarea indica su rutina) o una pestaña por rutina. Solo se puede marcar hoy o días anteriores.
- **Datos**: se guardan en la base de la biblioteca, separados por usuario.
- **IA**: desde el chat principal, el chat lateral (también en Finanzas) o Telegram podés hacer lo mismo que en la pantalla: consultar tu rutina, rachas, progreso e informes de este mes o de meses anteriores; crear, renombrar o eliminar rutinas; crear, editar, mover, pausar, reordenar, eliminar o recuperar tareas; marcar hábitos hechos (por ejemplo «ayer hice yoga») y ajustar metas. Cada cambio pide confirmación antes de guardarse. En Telegram, la rutina que ves y modificás es la del usuario de la biblioteca vinculado a tu cuenta.

### Imágenes del asistente

El asistente puede **generar** una imagen nueva a partir de lo que le pidas («dibujame un faro al atardecer», «un logo para mi emprendimiento con el texto "Luna"») y **editar** una imagen de la conversación: una que le mandaste o una que generó antes («ponela de noche», «sacale el fondo»). Usa el modelo Qwen-Image 2.1 Turbo, que corre en la placa de video de tu PC con Windows; nada sale de tu computadora.

- **Primera vez**: en **Configuraciones → Imágenes** tocá **Descargar 11,3 GB**. La descarga corre en segundo plano, se verifica archivo por archivo y, si se corta, **Seguir descarga** retoma desde donde quedó. **Borrar** quita los modelos de la PC.
- **En el chat de la app**: la imagen aparece dentro de la respuesta y queda guardada en el archivo del chat, así la volvés a ver al reabrirlo. Tocala para guardarla como PNG.
- **En Telegram**: el bot te manda la imagen como foto en el chat. No se guarda en la biblioteca, porque Telegram es efímero; podés pedir que la edite mientras siga en la conversación reciente.
- **Tiempo**: una imagen de 1024 × 1024 tarda alrededor de un minuto y una edición, cerca de dos. Mientras se genera, el progreso muestra «generando la imagen» y podés cancelar.
- **Celular**: en Android no se generan imágenes en el teléfono. Conectado como cliente a tu PC, el asistente las genera en ella.
- **Comando `/image`** (también `/imagen`): escribí `/image` y lo que querés ver, por ejemplo `/image un faro al atardecer`, en el chat de la app o por Telegram. Tu texto va tal cual al modelo de imágenes, sin pasar por el modelo de chat, que es el que a veces se niega o cambia el pedido; además es más rápido. Podés empezar con una proporción (`/image 16:9 un bosque`; también 1:1, 9:16, 4:3, 3:4, 3:2 y 2:3) y, si adjuntás una imagen, la edita según tu texto. Se cancela como cualquier pedido. El resultado se guarda y se envía igual que el resto de las imágenes del asistente.
- Notia no filtra el contenido de las imágenes, salvo uno: rechaza los pedidos sexuales o de desnudos con menores, por cualquier camino.
- Si la placa de video se queda sin memoria, el asistente te avisa; cerrá lo que la esté usando (por ejemplo Meeting) o pedí una imagen más chica. Si el modelo del chat corre en tu PC con Ollama, Notia lo saca de la placa mientras dibuja y Ollama lo vuelve a cargar para la respuesta siguiente.

### Terminal de la PC host

El asistente puede **ejecutar comandos en una terminal de la PC host** (la computadora donde corre Notia con tu biblioteca): ver archivos fuera de la biblioteca, correr un script, usar git, compilar, instalar algo, revisar procesos, el espacio en disco o la red.

- **Quién puede**: solo el **Owner**, desde los chats de la app (el principal, el lateral, el de una nota o el del tablero de tareas), desde su Telegram y desde las **Tareas IA**. Ningún otro usuario de la biblioteca, Meeting ni el **Task Manager publicado** pueden usarla, aunque el Owner haya iniciado sesión en la publicación.
- **Confirmación**: cada comando te pide confirmación y muestra el comando completo, la carpeta y el tiempo máximo. **Confirmar todos** deja que el resto de ese pedido corra sin volver a preguntar.
- **Estado en vivo con IA**: mientras el comando corre, un agente liviano (el mismo modelo, sin razonamiento ni herramientas) lee lo que va saliendo y escribe un estado corto: qué está haciendo, cuánto avanzó y si aparecen errores. El primero llega a los 15 segundos y después se renueva cada 30 segundos si salió algo nuevo. Un comando que termina antes no lo necesita.
- **En el chat de la app**: mientras corre, un recuadro **Terminal · ejecutando** muestra el comando, el estado de la IA y la salida en vivo, siguiendo las últimas líneas (si subís para leer, deja de seguir hasta que vuelvas abajo). Al terminar queda en el chat como un mensaje con el comando, cómo terminó, cuánto tardó y el final de la salida.
- **En Telegram**: cada comando tiene su propio mensaje que se actualiza con el estado de la IA y lo que va saliendo. Mientras corre, **cada minuto** el mensaje se vuelve a publicar abajo con el tiempo que lleva, el último estado y las últimas líneas, así te llega un aviso aunque la IA esté esperando un comando largo. Al terminar dice cómo terminó y en cuánto tiempo. Esto pasa aunque tengas apagado el mensaje de progreso.
- **En las Tareas IA** no hay estado en vivo, porque nadie mira la salida mientras corre; la tarea lee el resultado al terminar.
- **Shell y carpeta**: en Windows usa PowerShell (o `cmd` si el asistente lo elige); en Linux y macOS, `sh`. Si no se indica otra carpeta, corre en la carpeta de la biblioteca.
- **Tiempo máximo**: 10 minutos por defecto, hasta 4 horas si el asistente lo pide para algo largo. Al pasarlo, el comando se corta junto con todo lo que haya abierto. **Cancelar** el pedido también lo corta.
- **Sin entrada**: los comandos no pueden preguntarte nada; el asistente usa opciones como `-y` o `--yes`.
- **Uso de disco**: para saber qué ocupa lugar («¿qué me llena el disco?»), en lugar de un comando Notia mide la carpeta en una sola pasada: el tamaño de cada subcarpeta, los archivos más grandes y cuántas carpetas no pudo leer por permisos, sin fallar por ellas. Pide confirmación como un comando y tarda hasta 5 minutos; si no le alcanza, responde con lo medido.
- **Celular y clientes**: en Android no hay terminal. Conectado como cliente a tu PC, los comandos corren en la PC host; una copia sin conexión no ejecuta comandos.
- **Cuidado**: el asistente no ejecuta nada destructivo (borrar, formatear, desinstalar, reiniciar) si no se lo pediste explícitamente, pero revisá cada confirmación: el comando corre con tu usuario de Windows.

### Navegador web de la PC host

El asistente puede **manejar un navegador web en la PC host** con Playwright: abrir sitios, recorrerlos, hacer clic, completar formularios y buscadores, elegir opciones, leer páginas que se arman con JavaScript, ver algo de una cuenta tuya con la sesión iniciada y mirar una captura de la página.

- **Quién puede**: lo mismo que la terminal: solo el **Owner**, desde los chats de la app, su Telegram y las **Tareas IA**. Nunca en el **Task Manager publicado**, Meeting ni para otros usuarios. Las revisiones automáticas (la de cada hora, la que dispara un correo) no navegan.
- **Requisitos**: **Node.js 18 o más nuevo** y **Chrome o Edge** instalados en la PC host. La primera vez que el asistente abre el navegador, Notia instala Playwright con npm en su carpeta de datos (hace falta internet y tarda unos segundos más). No descarga otro navegador: usa tu Chrome, o Edge si no tenés Chrome.
- **Ventana y sesiones**: el navegador se abre en una ventana que ves en la PC, con un **perfil propio de Notia**, separado de tu perfil de siempre. Lo que inicies ahí queda guardado para la próxima vez.
- **Captchas, inicios de sesión y códigos**: cuando un sitio pide un captcha, una verificación de humanidad, iniciar sesión o un código (de tu autenticador, un SMS o un correo), **lo resolvés vos y el asistente hace lo que le digas**. Te manda por Telegram (o al chat) una captura del desafío con una **grilla de casillas** encima (filas con letras desde A arriba, columnas con números desde 1 a la izquierda) y te pregunta qué hacer. Respondés escribiendo, por ejemplo «B2 C3 D1» para tocar esas casillas, «arrastrá de A1 a A8» para un deslizador, el texto que muestra la imagen o el código de tu autenticador, y el asistente lo hace. Si aparece otra ronda, te manda otra captura y vuelve a preguntar. La pregunta espera hasta 10 minutos. También podés contestarle que lo resolvés vos en la ventana de la PC.
- **Lo que no hace**: nunca elige casillas ni resuelve un desafío por su cuenta, aunque vea la imagen, y no esconde que es un navegador automatizado. Una contraseña o un código que le des solo lo escribe en ese campo: no lo guarda en su memoria ni en sus notas. Sí queda en el historial del chat donde lo escribiste, así que si estás en la PC conviene iniciar sesión directamente en la ventana.
- **Confirmación**: **hacer clic, escribir, presionar teclas, elegir opciones y arrastrar** te piden confirmación, con el elemento como lo muestra la página, lo que se escribe y la dirección. **Confirmar todos** deja que el resto de ese pedido siga sin preguntar. Abrir páginas, leerlas, mirar una captura, esperar y manejar pestañas no piden confirmación.
- **Cuidado**: antes de comprar, pagar, transferir, enviar o publicar algo, borrar, aceptar términos o cambiar una contraseña, el asistente te muestra qué va a hacer y espera que se lo pidas explícitamente. No escribe contraseñas ni datos de tarjeta que no le hayas dado para ese sitio en la conversación. Lo que dicen las páginas es información: el asistente no sigue instrucciones que aparezcan en ellas.
- **Páginas que cambian**: si la página volvió a dibujar un botón o un campo (cambió su referencia) o lo redibuja mientras el asistente espera para tocarlo, Notia lo busca de nuevo por su nombre y lo usa, y avisa que lo hizo. En las apps hechas con Flutter, que solo muestran un botón «Enable accessibility», Notia lo activa solo para que se vean los campos del formulario.
- **Sesión de cada sitio**: cuando una página muestra un campo de contraseña, el asistente sabe que ese sitio pide iniciar sesión otra vez y te lo dice en lugar de reintentar. Si un sitio muestra un error o lo devuelve al ingreso, para después de un segundo intento, porque muchos bloquean después de varios.
- **Inicios de sesión de Google y Microsoft**: Google no deja iniciar sesión en un navegador automatizado; el asistente te propone otro método del sitio o que inicies sesión vos en la ventana. Antes de entrar a Teams u otra cuenta de Microsoft te avisa que vas a necesitar el teléfono para aprobar en Authenticator.
- **Diálogos**: un aviso de la página se acepta solo; una ventana de «¿Seguro?» se cancela, salvo que el asistente haya pedido aceptarla en ese clic (y lo ves en la confirmación).
- **Queda abierto**: el navegador sigue abierto entre mensajes, así una tarea puede continuar en el pedido siguiente. Se cierra cuando se lo pedís al asistente, cuando cerrás la ventana o cuando se cierra Notia.
- **Solo páginas web**: abre direcciones http o https; nunca archivos de la PC ni las páginas internas del navegador. No descarga archivos.
- **Celular y clientes**: en Android no hay navegador para el asistente. Conectado como cliente a tu PC, el navegador se abre en la PC host; una copia sin conexión no navega.

### Scripts de soporte del asistente

En la carpeta **`.agent/scripts`** de la biblioteca viven scripts Python que el asistente usa como herramientas.

- **Los escribe el asistente**: cuando un trabajo es difícil, largo o repetitivo y no tiene una herramienta para hacerlo (procesar muchos archivos, convertir formatos, cálculos o análisis de datos, armar un informe), o un script lo haría mejor que muchos pasos a mano, escribe uno y lo corre. Queda guardado para la próxima vez, y antes de escribir otro se fija si ya hay uno que sirve.
- **Los podés agregar vos**: poné un archivo `nombre.py` en esa carpeta (con el Explorador de Windows o el de Notia). El nombre va en minúsculas, con letras, números y `_`. Empezalo con un docstring que diga qué hace y qué parámetros recibe; el archivo `.agent/scripts/README.md` tiene un ejemplo completo. Ese README lo mantiene Notia: se lee pero no se edita ni se borra (como los archivos de `.agent/memory`), vuelve a aparecer si falta, y la carpeta `.agent/scripts` tampoco se puede borrar, renombrar ni mover. El asistente lo ve en el próximo pedido, sin reiniciar nada.
- **Cómo corren**: con el Python 3 de la PC host, en la carpeta de la biblioteca (también en la variable `NOTIA_LIBRARY`). Reciben los argumentos como JSON por la entrada estándar (`json.load(sys.stdin)`), y lo que imprimen es lo que lee el asistente, mejor en JSON. Se cortan a los 2 minutos, salvo que el asistente pida más (hasta 30). Los paquetes que no son de la biblioteca estándar de Python tienen que estar instalados.
- **Confirmación**: crear o cambiar un script te pide confirmación y te muestra el archivo entero. Correr un script que ya está en la carpeta no la pide, como cualquier otra herramienta: guardá ahí solo código en el que confiás. Un script tuyo, el asistente solo lo cambia si se lo pedís o si está roto.
- **Quién puede**: solo el **Owner**, en los chats de la app, Telegram y las Tareas IA, como la terminal. Nunca en el Task Manager publicado, Meeting ni para otros usuarios. Las revisiones automáticas pueden ver la lista, nunca correrlos.
- **Requisitos**: **Python 3** en la PC host (de python.org, con «Add python.exe to PATH»). En Android no hay scripts; conectado como cliente, corren en la PC host.

### WhatsApp

Notia se vincula a **tu WhatsApp como un dispositivo más** (igual que WhatsApp Web), desde la PC host, con Baileys. En **Configuraciones → WhatsApp** decidís, chat por chat, qué puede hacer el asistente y en cuáles responde solo.

- **Conectar**: tocá **Conectar**; aparece un código QR. En el teléfono abrí WhatsApp → **Dispositivos vinculados** → **Vincular un dispositivo** y escanealo. Al vincularse, la sección muestra tu nombre y número, y la lista de chats con los mensajes recientes que manda WhatsApp. Desde ahí Notia mantiene la conexión mientras está abierta (también minimizada a la bandeja) y se reconecta sola si se corta. **Desconectar** desvincula a Notia y borra la sesión y los chats guardados en la PC; los permisos de cada chat se conservan para la próxima vez.
- **Requisitos**: **Node.js 20 o más nuevo** en la PC host. La primera vez, Notia instala Baileys con npm en su carpeta de datos (hace falta internet). Tu teléfono sigue recibiendo las notificaciones como siempre.
- **Permisos por chat**: cada chat de la lista tiene tres interruptores. **Lectura** deja que el asistente lea el chat y mire las imágenes que llegaron. **Escritura** deja que el asistente le escriba y le mande archivos de la biblioteca. **Autorespuesta** hace que Notia conteste sola. Un chat sin permisos no existe para el asistente. El buscador encuentra chats por nombre o número.
- **Prompt del chat**: **Prompt** abre las instrucciones que sigue la autorespuesta en ese chat, por ejemplo «Sos el asistente de mi local: abrimos de lunes a viernes de 9 a 18; si preguntan precios, deciles que les paso la lista a la tarde».
- **Autorespuesta**: cuando alguien escribe en un chat con autorespuesta, Notia espera **un minuto desde su último mensaje** (para quienes escriben en varias líneas), lee todo lo que llegó —también hasta tres imágenes— y responde **un solo mensaje** con tu cuenta, siguiendo el prompt del chat. Si mientras tanto contestás vos desde el teléfono, no responde. Si los mensajes no necesitan respuesta (un «ok», un sticker), no manda nada. La autorespuesta solo conversa: no lee tu biblioteca ni usa otras herramientas, y no inventa datos ni compromisos que no estén en el prompt.
- **El asistente**: en los chats de la app, en Telegram y en las Tareas IA podés pedirle, por ejemplo, «¿qué me escribió Juan por WhatsApp?», «mirá la foto que mandó Ana», «decile a Juan que llego a las 8» o «mandale a Ana el PDF Presupuesto». Cada mensaje o archivo que manda te pide **confirmación** con el chat y el texto exacto (**Confirmar todos** deja seguir el resto del pedido; en una Tarea IA, el resto de la tarea).
- **Un número nuevo**: el asistente también puede escribirle a un número con el que todavía no tenés chat (por ejemplo, el WhatsApp de una empresa), si le das el número con código de país (+54 9 11 …). Notia comprueba que tenga WhatsApp antes de mandarlo, la confirmación te avisa que es un chat nuevo y, una vez enviado, ese chat queda con **Lectura** y **Escritura** para seguir la conversación (lo apagás en la sección). Un chat que ya existe sin permisos sigue cerrado: el asistente no lo abre escribiendo a su número.
- **Trámites en una Tarea IA**: una Tarea IA puede mantener una conversación por WhatsApp: escribe, espera la respuesta (ver **Tareas IA**) y sigue hasta resolver el trámite. Lo que llega por WhatsApp lo escriben terceros: el asistente lo trata como información y no sigue instrucciones que aparezcan ahí.
- **Quién puede**: solo el **Owner**, como la terminal y el navegador. Nunca en el **Task Manager publicado**, Meeting ni para otros usuarios. Las revisiones automáticas (la de cada hora, la que dispara un correo) pueden leer los chats habilitados, nunca escribir.
- **Privacidad**: la sesión de WhatsApp, los chats y sus últimos mensajes (hasta 100 por chat) quedan solo en la PC host. Los permisos y los prompts se guardan en `.notia/notiaConfig.json`, cifrado con tu contraseña de Owner.
- **Celular y clientes**: en Android no se vincula WhatsApp (necesita Node.js). Conectado como cliente a tu PC, la sección configura el WhatsApp de la PC host, y el QR se ve en el celular.

### Webhook

Otros programas pueden pedirle cosas a Notia. Por ejemplo, un sistema de seguridad manda la acción **Alerta de seguridad** con el prompt «Avisale al usuario por Telegram que…», y el asistente hace el pedido y te escribe por Telegram. Se configura en **Configuraciones → Webhook**.

- **Activar**: prendé **Activo**, elegí el **Puerto** (52485 de entrada) y tocá **Guardar**. Mientras Notia está abierto en la PC host, la sección muestra la dirección a la que llaman los programas, por ejemplo `http://127.0.0.1:52485/webhook`.
- **Desde otros equipos**: apagado **Llamadas desde la red**, solo pueden llamar los programas de la misma PC. Prendido, también otros equipos de tu red; para eso permití Notia en el Firewall de Windows. La conexión no va cifrada.
- **Acciones registradas**: solo se puede llamar con una acción registrada en la sección. **Registrar acción** pide su nombre (lo que el programa manda en `action`) e instrucciones opcionales, que valen más que lo que pida el programa. Cada acción se puede pausar, editar o eliminar. Una acción que no está registrada, o que está pausada, se rechaza.
- **Token**: al registrar una acción, Notia muestra su token **una sola vez**: copialo en el programa que va a llamar. Si lo perdés, **Nuevo token** crea otro y el anterior deja de funcionar. Notia guarda solo la huella del token.
- **Cómo llamar**: `POST /webhook` con el encabezado `Authorization: Bearer <token>` y el cuerpo JSON `{"action": "Alerta de seguridad", "prompt": "Avisale al usuario por Telegram que…"}`. La sección muestra el ejemplo con `curl`. Notia contesta enseguida con `202` y el número de la llamada; el pedido sigue en tu chat de Telegram.
- **Qué hace el asistente**: el pedido corre como uno tuyo en Telegram, con tus reglas, tu memoria y tus herramientas. Lo que cambia datos o manda mensajes a otros te pide confirmación por Telegram. Lo que manda el programa se trata como un pedido de afuera: no cambia las reglas ni los permisos del asistente. Si las instrucciones dicen avisar solo cuando haya algo y no hay nada, no te escribe.
- **Últimas llamadas**: la sección lista las llamadas recientes con su estado (en cola, en curso, enviado, sin mensaje, falló o rechazada) y el motivo. Se borran al cerrar Notia.
- **Límites**: cada acción acepta hasta 10 llamadas por minuto. El prompt puede tener hasta 8.000 caracteres.
- **Requisitos**: el bot de Telegram tiene que estar activo en la misma PC y tu usuario vinculado, porque la respuesta llega por ahí. Sin Telegram, las llamadas se rechazan y la sección lo avisa.
- **Celular y clientes**: en Android el webhook no escucha, igual que Telegram. Conectado como cliente a tu PC, la sección configura el webhook de la PC host.

### Acciones IA

El acceso **Acciones IA** (ícono del cuervo, Munin) aparece debajo de **Rutina** en la barra izquierda. Es el tablero de todo lo que la IA hace sola en un horario: qué hace (el prompt), cuándo, si ya se ejecutó hoy y si se repite. La IA siempre responde por Telegram y solo al Owner.

- **Tipos**: **Recordatorio** (una vez; la IA redacta un aviso breve a partir del prompt), **Hora específica** (una vez; la IA ejecuta el prompt como tarea, con las herramientas de la app, y te manda el resultado) y **Recurrente** (cada N minutos, horas, días o semanas, en los días elegidos y, para minutos u horas, dentro de una ventana opcional «desde–hasta»).
- **Tablero**: métricas del día (ejecutadas, pendientes, recurrentes activas y próxima ejecución), filtros por tipo con contador, buscador por nombre o prompt, una columna por tipo y el panel **Hoy** con todas las ejecuciones del día, pasadas y futuras, y la línea **AHORA**.
- **Estados**: Pausada, Falló (con **Reintentar**), Próxima, Ejecutada (o Enviado en recordatorios, o Reintentada), Pendiente y Programada. Una ejecución perdida porque Notia estaba cerrada figura como **Omitida**.
- **Formulario**: tipo, nombre, prompt, cuándo y un resumen con las próximas ejecuciones. **Probar ahora** ejecuta el prompt una vez sin guardarlo y la respuesta llega a Telegram con el prefijo «[Prueba]». Tocando una tarjeta se edita, se ve su historial y se puede eliminar (el historial se conserva).
- **Cómo corre**: cada ejecución es un pedido del Owner en su chat de Telegram, con sus reglas, su memoria, sus pensamientos y las herramientas de la app. Si algo necesita confirmación o una respuesta, te lo pregunta por Telegram. Una acción que pide «avisame solo si hay algo» puede no mandar nada.
- **Revisión de cada hora**: la revisión que antes hacía sola el agente autónomo ahora es una acción recurrente más, **Revisión de cada hora**, que Notia crea una vez en cada biblioteca (activa si tenías el agente autónomo prendido). Se puede editar, pausar o eliminar.
- **Dónde corre**: solo en el equipo donde está activo el bot de Telegram de la biblioteca. Si el servidor headless y la app de escritorio usan la misma biblioteca, el servidor tiene prioridad y cada ejecución ocurre una sola vez.
- **Horarios**: se leen en la zona horaria del lugar configurado en **Configuración → Clima** (Buenos Aires si no elegiste otro).
- **La IA las administra**: desde cualquier chat (el principal, el lateral, el de Finanzas, el del tablero de tareas, el de una nota) o por Telegram podés pedirle «recordame mañana a las 10 que pague la tarjeta», «todos los días a las 8 armame un resumen», «pausá la revisión de cada hora», «cambiá la de gastos a cada 2 horas», «borrá el recordatorio del contador», «ejecutá ahora el briefing» o «reintentá la que falló». La IA crea o cambia cualquier campo (tipo, nombre, prompt, fecha, hora, repetición, días, ventana, activa o pausada) y pide confirmación antes de guardar. Solo el Owner puede.

### Tareas IA

El acceso **Tareas IA** (ícono de flujo) aparece debajo de **Acciones IA** en la barra izquierda. Cuando le pedís al asistente algo largo o de muchos pasos (desde un chat de Notia o por Telegram), en lugar de hacerlo en la misma respuesta crea una **Tarea IA** y la trabaja aparte, mientras seguís usando Notia.

- **Cómo se crea**: solo la crea el asistente, y antes te pide confirmación. Usa una tarea cuando el pedido tiene un final («conseguí…», «hasta que…»), aunque tarde días; lo que se repite o un aviso a hora fija va en **Acciones IA**. Si una acción programada (como la revisión de cada hora) encuentra algo con final, te propone crear una tarea por Telegram en lugar de perseguirlo hora tras hora. Con **Confirmar todos** la tarea aplica sus cambios sin volver a preguntarte (las preguntas te las sigue haciendo).
- **Cómo trabaja**: tres agentes, cada uno con su prompt en `.agent/promps`: el **Planificador** (`planificador.md`) arma el plan paso a paso; el **Ejecutor** (`ejecutor.md`) hace cada paso hasta terminar; el **Review** (`review.md`) revisa el trabajo y lo aprueba o pide rehacer lo que falte, y entonces el Ejecutor sigue con esas correcciones. Notia crea esos tres prompts si no existen y podés editarlos.
- **Qué ves**: la lista de tareas (las que esperan tu respuesta arriba), y en cada una el flujo como diagrama de nodos (pedido, plan, pasos, revisiones y resultado, coloreados según su estado), el objetivo, el paso a paso con el informe de cada paso, las revisiones y el resultado final. Mientras el Planificador, un paso del Ejecutor o el Review trabajan, un recuadro **pensando** muestra en vivo lo que va razonando la IA («Ejecutor · paso 2 · pensando»), siguiendo el final del texto.
- **Esperar a otros o al tiempo**: si un paso depende de algo que todavía no pasó, la tarea queda **Esperando** en lugar de fallar, y sigue sola cuando pasa. Por ejemplo, le pasás un correo de una empresa y le pedís «gestioname con ellos que no tengo acceso»: la tarea les escribe, espera la respuesta en ese hilo, la lee, contesta o insiste, y vuelve a esperar hasta conseguir el acceso. Puede esperar:
  - un correo nuevo en un hilo, o uno que coincida con una búsqueda de Gmail (Notia revisa el correo cada 5 minutos sin usar la IA);
  - una respuesta por **WhatsApp** en un chat: sin usar la IA, Notia despierta la tarea **15 segundos** después del último mensaje que llega, con todo lo que mandaron (un bot suele mandar su menú en varios mensajes seguidos). Los menús con botones o listas llegan con sus opciones, y la tarea elige una como si la tocaras en el teléfono;
  - una fecha y hora;
  - algo que solo la IA puede comprobar, revisándolo cada cierto tiempo (por ejemplo, cada hora).

  Así podés pedirle, por ejemplo, «resolveme con Swiss Medical por WhatsApp el reintegro de la consulta del martes, mi número de socio es …» o «pedile a Movistar la baja del roaming de mi línea»: la tarea le escribe al WhatsApp de la empresa, espera cada respuesta, sigue los menús del bot con la opción exacta, pasa solo los datos que le diste para ese trámite y te pregunta antes de aceptar una oferta, pagar, dar de baja o cambiar un plan. Al final te cuenta qué respondieron y el número de gestión si lo dieron. El chat tiene que tener **Lectura** y **Escritura** en Configuraciones → WhatsApp; si todavía no tenés chat con la empresa, la tarea le escribe a su número oficial (con código de país) y ese chat queda habilitado.

  Cada espera tiene un plazo: 72 horas por defecto y hasta 30 días como máximo. Si vence sin novedades, la IA decide si insiste o termina el paso explicando qué falta. En la tarea ves qué espera, desde cuándo, cuándo vuelve a revisar y hasta cuándo, además de lo que hizo antes de cada espera. **Seguir sin esperar** la hace continuar enseguida. La espera sigue aunque cierres Notia, pero solo avanza con Notia abierta y con esa biblioteca elegida.
- **Aprobaciones**: si un paso necesita confirmación o te hace una pregunta, la tarea queda **Esperando aprobación** con las opciones **Confirmar**, **Confirmar todos**, **Proponer otra cosa** y **Cancelar**. Si la pediste por Telegram, también te llega por ahí con botones, y al terminar te manda el resultado.
- **Acciones**: **Cancelar** una tarea en curso, **Reanudar** una que falló, se canceló o quedó interrumpida porque Notia se cerró (sigue desde el paso donde quedó), y borrarla. El asistente también las administra: «¿cómo va la tarea de la wiki?», «cancelá la tarea», «revisala de nuevo». Al cancelar espera a que la tarea se detenga, así la lista la muestra cancelada enseguida.
- **Corregir una tarea sin crear otra**: pedile al asistente que le sume un dato o una URL, o que cambie el objetivo («la clave ya funcionó, solo falta avisar»): corrige el objetivo de la misma tarea, aunque esté trabajando o esperando, conserva los pasos hechos y planifica solo lo que falta. Si quedó trabada, pedile que la **reinicie**: empieza de cero con el mismo objetivo.
- **Sin plan no falla**: si el Planificador no devuelve pasos, la tarea usa los pasos escritos en el objetivo o trabaja el objetivo entero como un solo paso.
- **Cortes de la IA**: si la IA se corta o no contesta en un paso, o un paso termina sin informe, la tarea vuelve a hacer ese paso hasta dos veces más, sin repetir lo que ya cambió, y conserva los pasos hechos.
- **Por Telegram**: si vinculaste Telegram, las preguntas y aprobaciones de una tarea y su resultado te llegan ahí aunque la hayas creado en la app o en una Acción IA. Para contestar una pregunta con tus palabras, o proponer otra cosa en lugar de un cambio, respondé al mensaje de la tarea con **Responder**.
- **Cuánto duran**: cada tarea se borra sola una semana después de su último movimiento; mientras espera no se borra.

### Recetas

El acceso **Recetas** (ícono de un plato hondo) aparece debajo de **Rutina** en la barra izquierda. Es tu recetario: cada comida con su foto, sus ingredientes, la preparación y la información nutricional por porción.

- **Dónde se guardan**: cada receta es un archivo `.md` en la carpeta `recipes` de la biblioteca, con los datos en tablas y la foto adentro del mismo archivo. Se puede abrir como cualquier nota.
- **Tablero**: buscador por nombre o ingrediente, filtros por momento (**Desayuno**, **Almuerzo**, **Cena**, **Snack**), orden (más recientes, menos calorías, más proteína o por nombre) y una tarjeta por receta con calorías, tiempo y reparto de proteína, carbohidratos y grasas.
- **Detalle**: calorías por porción y qué parte son de una dieta de 2000 kcal, macros, fibra, azúcares, vitaminas y minerales con su porcentaje del valor diario. El sodio se muestra como parte del límite diario y se marca cuando es alto. Desde ahí se edita o se elimina (pide confirmación).
- **Nueva comida**: nombre, momento, tiempo, porciones, descripción, foto, ingredientes, pasos y los nutrientes que sepas. Al guardar, la IA revisa la comida y completa lo que falte, sobre todo vitaminas y minerales. Lo que escribiste se respeta. Sin foto, se muestra una ilustración del plato.
- **Sin repetidas**: si la comida ya está en el recetario (aunque la escribas un poco distinto), no se guarda otra igual y te ofrece **Ver la receta**.
- **Lo que comés también queda en el recetario**: si le mandás al asistente la foto de lo que comiste (con o sin texto) o le contás qué comiste, busca la receta. Si no existe, la arma con sus ingredientes y pesos estimados y la guarda (con la foto si la mandaste). Si ya existe, usa esa. En los dos casos también la carga en **Salud**. Para guardar una receta que todavía no comiste, pedíselo así: «guardá esta receta».
- **La IA la administra**: desde el chat principal, el lateral o Telegram podés pedirle «qué recetas tengo con lentejas», «cuánto hierro tiene el guiso», «cambiá las porciones del bowl a 3» o «borrá la receta de hummus». Cada cambio pide confirmación.

### Salud

El acceso **Salud** (ícono de un corazón con pulso) aparece debajo de **Recetas** en la barra izquierda. Es tu panel de peso, alimentación, agua, pasos, sueño y composición corporal. Cada usuario de la biblioteca ve solo sus propios datos.

- **Perfil**: fecha de nacimiento, sexo biológico, altura, peso y nivel de actividad. Con eso se calculan tu IMC, tu metabolismo basal, tu gasto diario y el agua que necesitás.
- **IMC**: el valor, su rango (bajo peso, normal, sobrepeso u obesidad), cuántos kilos te separan del rango normal para tu altura y dónde quedarías con tu objetivo.
- **Peso**: el gráfico de 30 días, 90 días, 1 año o todo, con la línea de tu objetivo. Podés cargar el peso de cualquier día y ver o borrar los registros. Tocá un punto del gráfico para ver su valor.
- **Peso objetivo y plan**: fijá el objetivo y el ritmo (0,25 a 1 kg por semana). **Generar plan con IA** arma tus calorías y macros diarios con recomendaciones; **Calcular sin IA** usa una fórmula. Las calorías nunca quedan por debajo de tu metabolismo basal. Sin plan, los objetivos son de mantenimiento.
- **Alimentación**: las comidas de cada día por desayuno, snack, almuerzo, merienda y cena, con calorías, proteínas, carbohidratos, grasas y fibra comparados con tu objetivo. Al agregar una comida podés tocar **Estimar con IA** para que complete los valores, o repetir una que ya cargaste.
- **Agua**: vaso de 250 ml, botella de 500 ml u otra cantidad, con el avance del día y los últimos 7 días.
- **Pasos**: los pasos de cada día (podés escribir «8.432»), con los de hoy, el promedio de la semana, la meta orientativa de 8.000 pasos y el gráfico de 30 días, 90 días, 1 año o todo, como el del peso. Podés cargar cualquier día y ver o borrar los registros.
- **Sueño**: cuánto dormiste cada noche, escrito como «7,5», «7:30» o «7 h 30», con la fecha del día en que te despertaste. Muestra lo de anoche, el promedio de la semana comparado con las 7 a 9 horas recomendadas y el gráfico con su histórico. También se cargan y borran por día.
- **Composición corporal**: cargá lo que te mide tu balanza (grasa, músculo, agua, huesos, grasa visceral, metabolismo, edad metabólica…). Se ve cómo se reparte tu peso, cada valor con su rango y la diferencia con la medición anterior. Lo que la balanza no da y se puede calcular se completa solo.
- **Desde cualquier chat**: también desde el chat de Finanzas, y el asistente cambia solo al módulo que haga falta.
- **Con la IA y por Telegram**: contale lo que comiste («almorcé dos empanadas de carne», «comí 500 g del guiso», «la milanesa pero con el doble de papas») o mandale la foto de tu plato, aunque no escribas nada. Busca la receta en **Recetas** (si no existe, la crea con ingredientes y pesos estimados) y la carga en tu alimentación con las calorías y macros de lo que comiste, ajustados si fue otra cantidad. También podés decirle «me pesé 82,4», «tomé un vaso de agua», pasarle los datos de la balanza, pedirle tu plan o preguntarle cuánto te queda por comer hoy. Cada cambio pide confirmación.
- Los rangos son referencias generales para adultos y no reemplazan una consulta profesional.

### Gimnasio

El acceso **Gimnasio** (ícono de una mancuerna) aparece debajo de **Salud** en la barra izquierda. Armás tus rutinas, entrenás con el reloj y el descanso, y ves cómo vienen tus músculos y tus semanas. Cada usuario de la biblioteca ve solo sus rutinas y su historial.

- **Panel de entrenamiento**: racha de semanas, entrenamientos de la semana contra los días que planificaste, calorías y tiempo; el cuerpo de frente y de espalda con cada músculo fatigado, recuperándose, recuperado o sin trabajar; un consejo sobre la próxima rutina; los días entrenados de las últimas 24 semanas (tocá uno para ver el detalle) y las calorías de los últimos 14 días.
- **El cuerpo es el tuyo**: se muestra masculino o femenino según el sexo de tu perfil en **Salud**.
- **Rutinas**: creá, renombrá, duplicá o eliminá rutinas, elegí sus días y sumá ejercicios desde la lista (buscador, grupos y **Solo con mi equipamiento**). Cada ejercicio arranca con 3 series de 10 repeticiones; ajustá el peso, las repeticiones (o los segundos), las series y el descanso.
- **Entrenar**: marcá cada serie como hecha y arranca solo el descanso, con el reloj del entrenamiento, pausa, ±15 s y saltar. **Terminar** pide confirmación y guarda el entrenamiento con sus series, minutos y calorías aproximadas. Un entrenamiento terminado no se reinicia con un toque de más: para volver a entrenar esa rutina, tocá **Empezar de nuevo**.
- **En el celular**: Gimnasio tiene su propia versión, con las secciones **Panel**, **Rutinas** y **Equipamiento** abajo. Rutinas muestra tus rutinas como tarjetas; al editar una, **Agregar ejercicio** abre la lista de ejercicios desde abajo, y al entrenar el descanso queda siempre a mano en la parte de abajo de la pantalla. La ficha de un ejercicio ocupa toda la pantalla.
- **Sigue donde estaba**: el entrenamiento en curso se guarda con cada serie, pausa o descanso, también lo que escribiste en un peso o en las repeticiones. Si se cierra la app o cambiás de pestaña, al volver a Gimnasio aparece el entrenamiento con el reloj corriendo. Al abrir la app con un entrenamiento de las últimas 6 horas, Notia va directo a él.
- **Equipamiento**: marcá con qué contás (o usá los atajos **Gimnasio completo**, **Casa con mancuernas** o **Sin equipamiento**) y la lista de ejercicios y tus rutinas avisan lo que te falta. Podés agregar equipamiento propio con foto.
- **Ficha del ejercicio**: video o imagen del movimiento, los pasos, los músculos principales y secundarios en el cuerpo, el equipamiento que necesita y las calorías por minuto. Desde **Editar** podés cambiar todo, subir un video, un GIF o una imagen, o crear ejercicios nuevos.
- **Dónde vive el catálogo**: cada ejercicio es una nota en `Gym/exercises` (con su imagen y, si tiene, el video al lado) y cada equipamiento una nota en `Gym/equipment`; el cuerpo son los SVG de `Gym/`. Las rutinas y el historial se guardan en la base de la biblioteca.
- **Con la IA y por Telegram**: pedile que te arme una rutina («armame una rutina de pecho para los lunes con lo que tengo en casa»), que cambie series o pesos, que empiece el entrenamiento y marque las series que hacés, que registre lo que entrenaste («hoy hice 45 minutos de piernas»), que te cuente cómo vienen tus músculos o tu racha, que busque ejercicios o que cree o corrija la ficha de uno (con la foto que le mandes). Cada cambio pide confirmación.
- Las calorías son una estimación de referencia: el gasto real cambia con tu peso, la carga y el ritmo.

### ColdPass

Gestor de credenciales cifradas integrado en Notia.

- Tus credenciales se almacenan en un archivo `ColdPass.md` dentro de cada librería, **cifrado con AES-256-GCM**.
- Se abre con la **contraseña del Owner**, la misma con la que iniciás sesión en Notia; no hay una contraseña aparte. Aunque tengas «Recordar sesión», ColdPass la pide cada vez que lo abrís y se bloquea al salir. Eliminar una credencial o importar un vault también la piden. Si cambiás la contraseña del Owner, ColdPass sigue abriéndose con la nueva.
- **Huella en Android**: se puede usar en tabletas y teléfonos con sensor de huella y Android 11 o superior. Con ColdPass abierto, tocá **Activar huella**, ingresá la contraseña del Owner y apoyá el dedo. Desde ese momento, al abrir ColdPass en ese dispositivo se pide la huella, con la opción de usar la contraseña. La huella se activa por dispositivo y por librería y no se sincroniza. También funciona si la tableta o el teléfono usa Notia en modo cliente de un host. Si cambiás las huellas registradas en Android o la contraseña del Owner, ColdPass vuelve a pedir la contraseña y hay que activar la huella de nuevo. Eliminar e importar siguen pidiendo la contraseña.
- La pantalla tiene la lista de credenciales con buscador y filtros (**Todas**, **Débiles**, **Antiguas**) y, al lado, el detalle: contraseña (mostrar y copiar), usuario, usuario secundario, sitio web, historial de contraseñas anteriores con su fecha y notas. En el teléfono, la lista y el detalle se ven de a uno.
- Cada contraseña muestra si es **Fuerte**, **Débil** (corta, fácil de adivinar o con el nombre, el sitio o el usuario adentro) o **Antigua** (más de un año sin cambiar), y ofrece **Generar nueva**.
- Al copiar una contraseña, Notia la borra del portapapeles a los 30 segundos si no copiaste otra cosa (Windows y Android). En Windows tampoco queda en el historial del portapapeles.
- **Editar y crear credenciales**: el formulario tiene nombre, sitio web, usuario, usuario secundario, contraseña y notas (hasta 500 caracteres). Mientras escribís la contraseña, una barra indica si es **Débil**, **Regular**, **Fuerte** o **Muy fuerte** y cuánto resistiría la fuerza bruta. **Deshacer cambio** vuelve a la contraseña guardada y, al guardar, la anterior pasa al historial. Las credenciales necesitan nombre y contraseña.
- **Generador de contraseñas**: dentro del formulario, con largo de 8 a 64 caracteres y opciones de mayúsculas, números, símbolos y sin caracteres ambiguos (0 O l 1 I). Cada contraseña generada trae al menos un carácter de cada tipo elegido. **Usar esta contraseña** la pone en el campo; se guarda recién con **Guardar cambios**.
- En el teléfono, el formulario ocupa toda la pantalla, con **Cancelar** y **Guardar** arriba.
- **Sincronización Bluetooth**: podés sincronizar tu bóveda de credenciales entre dispositivos de forma segura mediante Bluetooth Low Energy (BLE). El proceso incluye emparejamiento con PIN y autenticación de aplicación.
- **Extensión para Chrome** (carpeta `chrome-ext/coldpass`): completa usuario y contraseña en los sitios que ColdPass ya tiene, ofrece guardar los que no tiene y genera contraseñas con el generador de ColdPass.
  - **Requisitos**: Notia abierto en esta computadora en **modo Host** (Ajustes → Conexión). La extensión se conecta con `127.0.0.1` y el puerto del host (52480 por defecto); no sale a la red.
  - **Instalar**: en Chrome abrí `chrome://extensions`, activá **Modo de desarrollador**, tocá **Cargar descomprimida** y elegí la carpeta `chrome-ext/coldpass` del repositorio.
  - **Desbloquear**: tocá el ícono de Notia ColdPass e ingresá el puerto, el usuario Owner y la contraseña del Owner. Queda desbloqueada mientras Chrome esté abierto, aunque bloquees ColdPass en Notia; **Bloquear** la cierra. Al cerrar Chrome o Notia hay que volver a ingresar la contraseña.
  - **Completar**: en un sitio guardado (HTTPS, o `localhost`), usuario y contraseña se completan solos si los campos están vacíos. Tocando el campo de usuario o de contraseña aparece un menú con las credenciales del sitio; un sitio guardado como `google.com` también sirve para `accounts.google.com`. Los formularios de alta o cambio de contraseña no se completan.
  - **Generar**: en un campo de contraseña, el menú tiene **Generar contraseña**: 20 caracteres con mayúsculas, números y símbolos, sin caracteres ambiguos. Completa también el campo de repetir la contraseña.
  - **Guardar**: después de ingresar con un usuario y contraseña que ColdPass no tiene para ese sitio, aparece «¿Guardar en ColdPass?» con **Guardar** y **Ahora no**. La credencial se guarda con el nombre del sitio.

### Task Manager

Gestión de tareas por tableros, con tres vistas (Tablero, Lista y Management), Completadas, Canceladas y Pomodoro.

- **Encabezado**: las personas del tablero (sus avatares abren «Personas y proyectos»), **Proyectos**, **Refrescar**, el menú **Tablero ▾** (Editar tablero, Personas y proyectos, Nuevo tablero, Eliminar tablero) y **Nueva tarea** (tecla `N`), que crea «Nueva tarea» en la primera columna y la resalta.
- **Pestañas**: los tableros (el activo muestra cuántas tareas abiertas tiene y un engranaje para editarlo), **+** para crear uno, y a la derecha **Completadas**, **Canceladas** (sus contadores laten cuando cambian) y **Pomodoro**.
- **Buscar y filtrar**: la búsqueda (tecla `/`) filtra en vivo por título, descripción, personas y proyecto y dice cuántos resultados hay. Los filtros **Estado**, **Prioridad**, **Proyecto** y **Persona** se aplican a las tres vistas. La vista elegida se recuerda por tablero en este dispositivo.
- **Tablero**: columnas de 288 px en una fila con desplazamiento horizontal. Cada columna tiene su franja de color, su contador, **+**, **Plegar** y un menú (editar, mover a la izquierda o a la derecha, eliminar). Una columna se puede eliminar cuando no le quedan tareas abiertas: las finalizadas o desestimadas que estaban en ella quedan «Sin grupo» en Completadas y Canceladas. Al mover una tarea a otra columna, sus subtareas van con ella. Las columnas vacías arrancan plegadas. Arrastrá una tarjeta a otra columna o a otro lugar de la misma (en táctil, mantenela presionada); soltarla en una columna plegada la abre. **Nueva columna** agrega una al final.
- **Tarjeta**: prioridad (tocala para elegir entre Atrasado, Urgente, Alta, Media y Baja, también con las teclas 1 a 5; «Atrasado» es la más urgente, para lo que ya se pasó de su fecha), estado «En progreso · m:ss» con el cronómetro en vivo o «Bloqueada», proyecto (elegí uno o escribí un nombre nuevo para crearlo), título (tocalo para abrir el ticket; doble clic en el título o en el detalle abre un modal para leer y editar su contenido con el editor de las notas; pasá el mouse para ver el título entero si está recortado; **Renombrar** desde el menú ⋯), círculo de tiempo (dedicado sobre estimado: se pone coral si te pasaste; tocalo para sumar o restar horas y estimar con atajos de 1 a 16 h), descripción, motivo de bloqueo, subtareas con su avance, personas asignadas (solo las del tablero), comentario, **Iniciar**, **Pausar** y **Finalizar**.
- **Menú ⋯ de la tarea**: iniciar, pausar, finalizar o desbloquear; renombrar; prioridad; **Mover a** otra columna; asignar personas; cambiar proyecto; estimar tiempo; subtareas; duplicar (copia con «(copia)», las mismas personas y dependencias, las subtareas sin marcar y sin horas); motivo del bloqueo; editar la nota; agregar un comentario; **Desestimar** (pasa a Canceladas) y **Eliminar**.
- **Deshacer**: finalizar, desestimar y eliminar muestran un aviso abajo con **Deshacer** durante 5 segundos; deshacer devuelve la tarea tal como estaba.
- **Cronómetro**: **Iniciar** pone la tarea en progreso y empieza a contar; **Pausar**, finalizar o moverla a **Bloqueado** suman ese tiempo a las horas dedicadas. Mover una tarea a la columna **Bloqueado** la bloquea y sacarla de ahí la deja pendiente.
- **Lista**: cada columna es un grupo plegable (independiente del tablero) con una fila por tarea: proyecto, personas, estado, tiempo, prioridad y acciones; las subtareas se despliegan debajo de su tarea. Las filas se arrastran entre grupos.
- **Management**: Notia arma un plan con fecha y hora para cada tarea activa (todas menos Anotadores), respetando las dependencias, cuándo puede empezar cada columna (Sprint siguiente desde el próximo sprint, Próximo Q desde la próxima división) y que cada persona haga una sola tarea a la vez. Muestra el rango (los sprints con tareas o el Q completo), la carga del equipo, quién está más cargado y cuándo termina lo planificado; un Gantt por tarea o por persona con la ruta crítica en coral, la holgura punteada y la espera por una persona en violeta; la red CPM; y la tabla CPM, donde se editan las predecesoras (las que crearían un ciclo aparecen deshabilitadas). Pasar el mouse (o tocar) una tarea la resalta en los tres.
- **Personas y proyectos**: un panel lateral para agregar personas al tablero (solo ellas se pueden asignar) y proyectos con su color. Quitar a alguien lo saca de todas sus tareas; eliminar un proyecto deja sus tareas sin proyecto. El propietario no se puede quitar.
- **Editar tablero**: nombre, color, contexto (con «+ Nuevo»), la jornada (franja horaria de pasos de 30 minutos, con atajos según el contexto, y los días activos), los sprints (de 1 a 8 semanas, día de inicio) y las divisiones del año (de 1 a 12, prefijo y mes en que empieza el año), con una vista previa de los sprints y del año. La jornada define las horas por día del plan; los sprints, las fechas de Sprint actual y Sprint siguiente; las divisiones, Próximo Q y el selector de Management.
- **Completadas y Canceladas**: una tabla de las más recientes, con búsqueda y filtros por grupo, prioridad, proyecto y persona; seleccioná varias para **Recuperar** (vuelven a su columna como pendientes) o **Eliminar**. Completadas carga de a 20.
- **Comentarios**: cada comentario se agrega al final del ticket como `## Comentario - DD/MM/YYYY HH:MM - Autor`, con la hora local y el nombre del usuario.
- **Persistencia transparente**: cada tarea se guarda como un archivo Markdown con metadatos (frontmatter) dentro de la carpeta del tablero correspondiente.
- **Agente contextual**: el chat lateral conoce el panel activo de Task Manager pero no adjunta todos los tickets. Las búsquedas y lecturas quedan limitadas al tablero o panel visible; para consultar otro contexto primero hay que cambiar a ese panel. Usa RAG local para consultas generales y lee archivos completos bajo demanda mediante tool calling nativo de Ollama. Cuando recupera o lee un ticket padre, incorpora automáticamente las subtareas declaradas en `childs` y su contenido, de forma recursiva, para que la respuesta no pierda sus seguimientos.
- **Filtros y organización asistida**: puede buscar por estado, prioridad, grupo, fechas, tags y texto de metadata, y actualizar dependencias y checklist controlados en el frontmatter sin cargar cuerpos innecesarios.
- **Edición asistida y confirmada**: el agente puede crear tickets, reemplazar su contenido Markdown, agregar, corregir o eliminar comentarios, agregar subtareas, mover tickets de grupo y cambiar estado o prioridad. También maneja todo lo del tablero con sus mismas reglas: estimar y cargar horas dedicadas, iniciar y pausar el cronómetro, asignar personas (solo las del tablero) y proyectos, escribir el motivo de bloqueo, encadenar tareas sin ciclos, marcar subtareas, duplicar, agregar o quitar personas y proyectos del tablero, cambiar la jornada, los sprints y las divisiones, y fijar el tipo de cada columna. Para fechas, carga del equipo, holguras y ruta crítica consulta el plan de Management que calcula Notia, en vez de calcularlas. La confirmación nombra las tareas por su título y dice qué regla se aplica (por ejemplo, que mover a «Bloqueado» bloquea la tarea). Un comentario corregido conserva su fecha y su autor. El Owner puede cambiar cualquier comentario y los demás usuarios solo los propios. La confirmación cita el comentario completo. También puede consultar los grupos en el orden del tablero, crearlos, renombrarlos, cambiarles el color, reordenar las columnas y eliminarlos únicamente cuando no tengan ningún ticket asignado. Los índices del tablero (`*TaskIndex.md`) no los edita como documentos: los cambia siempre con las herramientas de Task Manager, que también actualizan los tickets de cada grupo. Consulta las opciones válidas del tablero y, ante cualquier dato faltante, definición imprecisa o coincidencia ambigua, pausa para preguntar en vez de inventar. Toda interacción pendiente aparece en una tarjeta dentro del chat: si encuentra varias opciones, cada alternativa se presenta como una opción clickeable; si necesita autorización, muestra los valores concretos, una vista previa del contenido y las acciones **Confirmar** y **Cancelar**. El runtime impide modificar entidades ambiguas hasta resolver la selección. Rechazar una autorización garantiza que no se escriba nada y una aprobación solo autoriza esa operación individual.
- **Planes para operaciones compuestas**: cuando un pedido requiere dos o más escrituras, el agente crea primero un TO-DO visible dentro del chat. El usuario debe aprobarlo antes de comenzar o puede elegir **Sugerir cambios**, escribir la corrección en el compositor y revisar una nueva versión. Cada operación se ejecuta por separado y en orden, conserva su propia confirmación y actualiza el paso como pendiente, en curso, completado o bloqueado. Un rechazo o error detiene el avance del plan. En documentos y biblioteca se usa el plan general; Task Manager conserva su alias específico.
- **Resúmenes por persona**: cuando se solicita una vista completa por responsables, el agente inspecciona todos los tickets del panel, releva las atribuciones explícitas tanto de los metadatos como de los detalles y evita agrupar el trabajo de distintas personas bajo el primer nombre encontrado.
- **Búsqueda de personas**: los resultados relevantes se diversifican entre archivos para que un historial con muchas menciones no desplace otros tickets coincidentes. La cantidad informada corresponde a rutas de tickets únicas, no a comentarios o estados dentro de un mismo archivo.
- **Panel adaptable**: el borde izquierdo del chat lateral permite ajustar su ancho con arrastre o teclado y conserva la medida elegida entre sesiones.
- **Encabezado y compositor del chat lateral**: arriba están el agente que responde, el título del chat abierto, el historial de chats, «Nuevo chat» y cerrar. El historial agrupa los chats en Fijados, Hoy, Ayer, Esta semana y Anteriores, muestra con qué agente habla cada uno, se puede buscar y permite fijar, renombrar o eliminar cada chat (Ctrl+H lo abre y Ctrl+N empieza un chat nuevo). Un chat elegido del historial queda abierto, con su agente, aunque no sea el de la nota actual. El chat vacío muestra con quién hablás y sugerencias que se envían con un toque. El compositor muestra como chip el archivo o la vista que se suma de contexto, junto con los adjuntos, y tiene adjuntar, dictar, el modelo (abre la configuración de IA) y enviar.
- **Colaboración en tiempo real**: la publicación LAN permite configurar entre 1 y 64 sesiones autenticadas y conexiones WebSocket concurrentes (64 por defecto). Las mutaciones del host y de las URL publicadas se distribuyen en ambas direcciones con revisión, reintento idempotente y detección de conflictos; una edición que quedó vieja pide recargar y no pisa el cambio de otra persona. Si se corta la red, la URL reconecta y recupera eventos por cursor o solicita un snapshot nuevo; al ocultar la pestaña pausa socket y reintentos y resincroniza al volver. Revocar un dispositivo cierra solo sus conexiones activas.

---

### En el celular

Cuando un módulo tiene poco ancho (un teléfono, una tableta angosta o la ventana con el Explorador o el Asistente abiertos), cambia a su versión para celular, pensada para usar con el dedo. Además de Finanzas y Gimnasio, la tienen:
- **Inicio:** fecha y saludo arriba, tarjeta del clima (al tocarla muestra el pronóstico en una hoja), cuadro para preguntarle al asistente, botones **Nueva nota** y **Grabar reunión**, y las tarjetas una debajo de otra. Mientras grabás, una píldora abajo muestra el tiempo y **Detener**. En una tableta con el Explorador abierto, las tarjetas pasan a una columna para que nada se superponga.
- **Agenda:** el mes con la leyenda de feriados y el día elegido, una tira con los días de la semana y la línea del día en bloques de 15 minutos, el anotador rápido y los próximos eventos deslizándose de costado.
- **Meeting:** cada paso en una pantalla con la acción principal fija abajo (el botón de grabar, Marcar/Finalizar/Pausar o Guardar como nota); el contexto para la IA, la nueva marca, las Notas IA completas, las notas rápidas y los momentos se abren en hojas, el asistente en vivo alterna Notas IA y Respuestas, y la reunión terminada separa Transcripción e IA en pestañas.
- **Chat IA:** encabezado con el historial (un cajón con la vista previa de cada chat), el contexto y un chat nuevo; la barra de agentes, y hojas para el contexto, adjuntar, agregar agentes y elegir la dinámica. El compositor queda sobre el teclado.
- **Graph View:** tocar una nota la selecciona y abre una hoja con **Abrir nota**, el grafo local y sus conexiones; la búsqueda y **Vista y filtros** tienen su propia pantalla. Se usa con el pellizco para el zoom, se arrastra un nodo para fijarlo y se toca dos veces para liberarlo.
- **ColdPass:** la lista de contraseñas con búsqueda y filtros, y cada credencial a pantalla completa con **Copiar contraseña** a todo el ancho.
- **Recetas:** lista o cuadrícula con filtros por momento del día y orden, cada receta a pantalla completa con pestañas Nutrición, Ingredientes y Preparación (podés tildar ingredientes y pasos mientras cocinás) y el formulario a pantalla completa.
- **Salud:** pestañas Hoy, Comidas, Peso, Actividad (pasos y sueño) y Cuerpo; en Hoy, las calorías de hoy, el peso del último mes y los pasos y el sueño llevan al detalle. Un botón flotante agrega una comida o registra una medición, y los formularios se abren como hojas desde abajo.

El botón Atrás de Android todavía no cierra las hojas ni las pantallas completas: usá el botón de cerrar o volver de cada una.

## 🧩 Consumo de Funcionalidades

> Esta sección describe, para cada módulo funcional, **qué hace**, **cuándo usarlo**, los **pasos para consumirlo**, las **entradas esperadas**, las **salidas/resultados** y los **errores comunes** con su resolución. Se expresa en lenguaje funcional (orientado a analistas y usuarios finales).

### Inicio: el tablero de la biblioteca

| Campo | Descripción |
|---|---|
| **Qué hace** | Reúne en una pantalla la agenda de la semana, las tareas urgentes y el Pomodoro, el mes de Finanzas, los hábitos de hoy, el anotador del día, lo último que usaste y el clima, y permite actuar sobre ellos sin abrir cada módulo. |
| **Cuándo usarlo** | Al empezar el día o para ver rápido qué tenés pendiente. |
| **Pasos para consumir** | 1. Tocar **Inicio** (la casa) en la barra izquierda. 2. Marcar hábitos o pendientes del anotador con su casilla. 3. Tocar un día de la agenda para filtrar sus eventos. 4. Iniciar, pausar o reiniciar el Pomodoro. 5. Escribir en **Preguntale al asistente** y tocar la flecha para mandarlo al chat lateral. 6. Tocar **Abrir** en una tarjeta para ir a su módulo. |
| **Entradas esperadas** | Pendientes de hasta 200 caracteres en el anotador; una pregunta cualquiera en el cuadro del asistente. |
| **Salidas / Resultado** | Cada cambio se guarda en su módulo (Agenda, Rutina o Task Manager) y el tablero se vuelve a leer. La pregunta aparece en un chat nuevo del chat lateral con el agente elegido. |
| **Errores comunes** | **Una tarjeta muestra un error en rojo**: ese módulo no se pudo leer (por ejemplo, la biblioteca perdió su permiso en Android); las demás tarjetas siguen. **«Grabar» deshabilitado**: el modelo de voz todavía se está preparando o el dictado está desactivado en Configuraciones. **Dólar en «—» y «Sin conexión»**: no se pudo consultar la cotización. **«Clima no disponible»**: no se pudo consultar Open-Meteo; tocá el chip para reintentar. **El clima es de otra ciudad**: elegí la tuya en **Configuraciones → Clima**. |

### Librerías y Explorador de Archivos

| Campo | Descripción |
|---|---|
| **Qué hace** | Indexa una carpeta local del filesystem y la presenta como un árbol jerárquico interactivo. Permite navegar, crear, copiar, mover, renombrar, eliminar y buscar archivos. |
| **Cuándo usarlo** | Siempre que necesites organizar, acceder o modificar tus notas y documentos dentro de una carpeta de trabajo. |
| **Pasos para consumir** | 1. Abrir Notia. 2. En el panel izquierdo, clic en **"Administrar librerías"** (footer). 3. Clic en **"Agregar librería"**. 4. Seleccionar una carpeta del filesystem (escritorio) o conceder permisos SAF (Android). 5. La carpeta aparece en el Explorador. |
| **Entradas esperadas** | Una ruta absoluta de carpeta (escritorio) o una URI de árbol SAF (Android). En Android, también un nombre descriptivo para la librería. |
| **Salidas / Resultado** | Árbol de archivos renderizado en el panel izquierdo. Los archivos y carpetas se listan con íconos según tipo. En escritorio, el árbol se actualiza automáticamente ante cambios externos. |
| **Errores comunes** | **"El selector de carpetas tardó demasiado. Intenta nuevamente."**: DocumentsUI no respondió dentro de 60 segundos; el modal queda disponible para reintentar. **"No se pudo acceder a la carpeta"** o **"Could not resolve Android directory."**: verificá que `selection.androidTreeUri` no esté vacío y reseleccioná la carpeta mediante Android SAF usando la versión actual de la aplicación. **Error al escribir la configuración**: SAF rechazó la escritura, se revocó el permiso o falló la escritura del contenido inicial; el plugin realiza _best-effort cleanup_ del documento creado y la librería no se agrega, así que podés volver a seleccionar la carpeta. Los mensajes de error ahora incluyen el detalle original de SAF (por ejemplo, `Could not create file. No se recibió una URI SAF válida.`) para facilitar el diagnóstico. **"No se pudieron guardar los cambios pendientes antes de agregar la libreria."**: guardá o corregí el documento pendiente y reintentá. **"El árbol está vacío"**: la carpeta seleccionada realmente no tiene archivos, o el path es incorrecto. |

### Crear una Nota Markdown

| Campo | Descripción |
|---|---|
| **Qué hace** | Crea un archivo de texto con formato Markdown dentro de una carpeta del Explorador. |
| **Cuándo usarlo** | Cuando querés documentar información estructurada con formato enriquecido, wikilinks, frontmatter y propiedades. |
| **Pasos para consumir** | 1. Seleccionar una carpeta en el Explorador. 2. Clic derecho → **"Nueva nota"** (o usar el botón **Nueva nota** del encabezado del Explorador, o `Ctrl + N`). 3. Ingresar el nombre del archivo. 4. Presionar Enter o clic fuera. 5. El archivo se crea y se abre automáticamente en una pestaña. |
| **Entradas esperadas** | Nombre del archivo (string, sin caracteres especiales `/`, `\`, `.`, `..`). El sistema agrega automáticamente la extensión `.md`. |
| **Salidas / Resultado** | Archivo `Nombre.md` creado en el filesystem. Pestaña abierta con el editor Markdown listo para edición. |
| **Errores comunes** | **"Nombre inválido"**: contiene caracteres prohibidos o está vacío. Solución: usar solo letras, números, espacios, guiones y guiones bajos. |

### Editar una Nota Markdown (Wikilinks)

| Campo | Descripción |
|---|---|
| **Qué hace** | Permite vincular notas entre sí mediante la sintaxis `[[Nombre de Nota]]`, creando un grafo de conocimiento bidireccional. |
| **Cuándo usarlo** | Cuando querés relacionar conceptos, ideas o documentos entre sí para navegación rápida y descubrimiento de conexiones. |
| **Pasos para consumir** | 1. Abrir una nota Markdown. 2. En cualquier parte del texto, escribir `[[Nombre de otra nota]]`. 3. Notia resaltará el wikilink y mostrará sugerencias mientras escribís. 4. Hacer clic en el wikilink para abrir la nota destino en una nueva pestaña. 5. Si la nota destino no existe, Notia ofrecerá crearla. |
| **Entradas esperadas** | Texto con patrón `[[nombre de nota]]`. El nombre debe coincidir (case-insensitive) con un archivo `.md` existente en la librería. |
| **Salidas / Resultado** | Enlace bidireccional activo. Al hacer clic se abre la nota destino. El Graph View utiliza estos enlaces para construir el mapa de relaciones. |
| **Errores comunes** | **Wikilink rojo/quebrado**: la nota destino no existe. Solución: crear la nota destino o corregir el nombre. |

### Enlaces Secuenciales — Page Links (`nextPage` / `previousPage`)

| Campo | Descripción |
|---|---|
| **Qué hace** | Vincula notas Markdown en una secuencia ordenada mediante las propiedades de frontmatter `nextPage` y `previousPage`. Útil para navegar entre capítulos, pasos de un proceso o entradas de un diario. |
| **Cuándo usarlo** | Cuando necesitás que varias notas estén conectadas en un orden específico y que el explorador las agrupe como un bloque secuencial. |
| **Pasos para consumir** | 1. Abrí una nota Markdown; el panel **Propiedades** está arriba del texto. 2. Encontrá la propiedad `nextPage` (se crea automáticamente al abrir una nota si no existe). 3. Tocá **Vincular nota…** (o el lápiz, si ya tiene una nota). 4. Escribí parte del nombre y elegí la nota de la lista, o tocá **Crear nota «…»** para crearla junto a la actual. 5. Notia actualizará automáticamente la nota destino para que tenga `previousPage: [[Nombre de la nota actual]]`. 6. Repetí el proceso para `previousPage` si es necesario. |
| **Entradas esperadas** | Un wikilink válido: `[[nombre-de-archivo.md]]`. Se aceptan referencias sin extensión (ej. `[[6-10]]`) que se resuelven automáticamente a `.md`. |
| **Salidas / Resultado** | Las dos notas quedan vinculadas bidireccionalmente. El **Explorador** renderiza las notas conectadas con una línea vertical que las agrupa como un bloque. Los bloques y las notas sueltas se ordenan juntos por la fecha de creación (`createdAt`) de su primera nota, así una nota encadenada con notas de otras carpetas conserva su lugar en la suya. |
| **Errores comunes** | **Ciclo detectado**: si A → B → C, intentar que C apunte a A es rechazado. Solución: mantener una cadena lineal sin ciclos. **Link roto**: si `nextPage` apunta a un archivo inexistente, se ordena como nota suelta por fecha. Solución: verificar que el archivo exista. |

### Graph View

| Campo | Descripción |
|---|---|
| **Qué hace** | Visualiza todas las notas Markdown de la librería como nodos y los wikilinks entre ellas como conexiones, permitiendo navegación visual interactiva. |
| **Cuándo usarlo** | Cuando querés explorar visualmente las relaciones entre tus notas, encontrar notas aisladas o descubrir clusters de conocimiento. |
| **Pasos para consumir** | 1. Asegurate de tener notas Markdown con wikilinks en la librería. 2. En el **Icon Rail** (barra lateral izquierda), seleccionar **"Graph view"**. 3. Esperar a que se cargue el grafo (puede tomar segundos en bibliotecas grandes). 4. Usar zoom y paneo para explorar, o **Encuadrar** para ver todo. 5. Tocar un nodo para seleccionarlo y ver su detalle; **Abrir nota** la abre en una pestaña y **Sumar al chat** la agrega al contexto del chat. 6. Usar el buscador para encontrar notas por título o contenido y tocar un resultado para ir a su nodo. 7. Usar **Ver grafo local**, los chips de etiqueta y los controles de abajo para filtrar. |
| **Entradas esperadas** | Librería activa con al menos un archivo Markdown. No requiere entrada manual del usuario. |
| **Salidas / Resultado** | Canvas 2D interactivo con nodos (títulos de notas) y líneas de conexión (wikilinks). Al tocar un nodo se muestra su detalle; **Abrir nota** la abre en pestaña. |
| **Errores comunes** | **"El grafo está vacío"**: no hay archivos Markdown en la librería. Solución: crear notas Markdown. **"Lentitud"**: bibliotecas con miles de notas pueden tardar en construir el modelo. El archivo `linkCache.md` dentro de `.notia/` acelera la vista previa del grafo y se regenera automáticamente en segundo plano; si aún se siente lento, considerá dividir la librería en partes más pequeñas. |

### AI Chat con Ollama

| Campo | Descripción |
|---|---|
| **Qué hace** | Permite conversar con Ollama local o Cloud, según la URL configurada, con las reglas (`rules.md`) y la memoria (`memory.md`) del agente, contexto de archivos de la librería y análisis de imágenes. |
| **Cuándo usarlo** | Cuando necesitás asistencia de IA para redactar, resumir, analizar imágenes o consultar sobre el contenido de tus notas. |
| **Pasos para consumir** | 1. En Notia, abrir **Settings → IA**. 2. Dejar `https://ollama.com` para Ollama Cloud o configurar una URL local compatible con el chat. 3. Si se usa Cloud, ingresar la API Key sin compartirla en el prompt. 4. Hacer clic en **"Verificar conexión"**. 5. Seleccionar un modelo de la lista. 6. En el Icon Rail, abrir **"AI Chat"**. 7. Escribir un mensaje y presionar Enter. 8. Opcional: adjuntar archivos de la librería como contexto (modo **Directo** para contenido completo, **Referencia** para lista de nombres/rutas). 9. Opcional: abrir **Adjuntar archivo → Seleccionar archivo**, elegir uno o varios archivos locales y quitar individualmente los que no quieras enviar. 10. Opcional: cancelar una respuesta en curso con el botón **Cancelar** del compositor lateral o **Detener respuesta** en la vista Chat IA. |
| **Entradas esperadas** | Texto del mensaje (string, límite flexible ~30k caracteres de contexto acumulado). Opcional: uno o varios archivos locales de imagen, PDF o texto; cada archivo admite hasta 40 MB, los textos hasta 10 MB (más de 400.000 caracteres se leen por partes) y los PDF hasta 24 páginas. No hay límite ejecutable de tamaño total del lote ni cantidad máxima de imágenes. Opcional: archivos de la librería como contexto. |
| **Salidas / Resultado** | Respuesta de texto del modelo de IA. Durante la generación, el pensamiento ocupa un bloque de altura fija que avanza hacia el fragmento más reciente y el hilo mantiene visible la parte inferior de la respuesta. Si todavía no hay una sesión seleccionada, el primer envío crea y muestra el chat inmediatamente en el panel lateral. La sesión se guarda automáticamente con un título generado por IA. Los mensajes de usuario conservan la metadata de sus adjuntos en el Markdown del chat; al recargar, los nombres vuelven a mostrarse y los adjuntos incluidos en la ventana de contexto pueden reutilizarse en una consulta de seguimiento. Las reglas y memorias del agente se cargan y se guardan (cuando el agente usa sus herramientas) en las superficies persistentes y en Telegram vinculado al Owner; Meeting, la publicación y Telegram vinculado a otro usuario mantienen políticas sin memoria. |
| **Errores comunes** | **"No se pudo conectar con Ollama"**: el proveedor no responde o la URL es incorrecta. Solución: verificar el endpoint en Settings; para una instalación local, confirmar que Ollama esté ejecutándose. **"No se pudo procesar el archivo"**: el tipo, tamaño, contenido, límite de caracteres o cantidad de páginas no es compatible; elegir archivos de imagen/PDF/texto dentro de los límites. **"La búsqueda web no está disponible"**: la búsqueda pública requiere Ollama Cloud y una API Key válida; el chat local puede seguir funcionando. **"La IA no devolvió contenido"**: reintentar o cambiar de modelo. **"El modelo seleccionado no admite imágenes"**: elegir un modelo con capacidad de visión en Settings → IA. |

### Agentes del Chat IA

| Campo | Descripción |
|---|---|
| **Qué hace** | Hace que respondan uno o varios agentes (prompts de `.agent/promps/`) en lugar de Notia, en rondas, guiados opcionalmente por una dinámica de `.agent/dynamics/`, con los permisos y el contexto permanente del chat. |
| **Cuándo usarlo** | Para comparar perspectivas, pedir un debate o un análisis entre varios roles, o encadenar el trabajo de varios agentes sobre la librería. |
| **Pasos para consumir** | 1. Abrir el **Chat IA** y el panel **Contexto**. 2. En **Agentes**, tocar **Agregar agente** y elegir hasta seis. 3. Opcional: elegir una **Dinámica**, escribir un **Contexto permanente** y ajustar los **Permisos**. 4. Enviar el mensaje. 5. Detener con el botón del compositor si hace falta. |
| **Entradas esperadas** | Agentes y dinámicas en Markdown con contenido; el frontmatter se ignora y su campo `description`, si existe, se muestra en la lista. El contexto permanente admite hasta 20.000 caracteres. |
| **Salidas / Resultado** | Mensajes de cada agente con su nombre e iniciales, guardados en el chat. Entre una y cuatro rondas automáticas por mensaje; la dinámica puede nombrar agentes, pedir que hablen todos o pedir esperar al usuario. |
| **Errores comunes** | **«Un chat puede tener hasta seis agentes»**: quitá uno antes de sumar otro. **«La dinámica del chat no es válida»** o un agente marcado **Vacío o ilegible**: revisá el archivo en `.agent/`. Un agente cuyo archivo se borró queda en la lista con el aviso «El archivo del agente ya no existe» y no habla. Si un agente falla, lo que dijeron los anteriores queda guardado y se muestra el error. |

### Finanzas: carga por el asistente y «Para revisar»

| Campo | Descripción |
|---|---|
| **Qué hace** | El asistente carga, corrige y elimina los datos financieros a partir de lo que le mandás (fotos de tickets, recibos, facturas y resúmenes, o un mensaje). Al guardar une solo lo que es claramente lo mismo, te lo cuenta y deja lo dudoso en **Para revisar** como una pregunta. La pantalla de Finanzas muestra los resultados. |
| **Cuándo usarlo** | Cada vez que pagás algo y querés registrarlo, cuando llega el resumen de la tarjeta ya pagado, al cobrar el sueldo, al comprar o vender dólares del ahorro, o para corregir, unificar o borrar algo cargado. |
| **Pasos para consumir** | 1. Abrir el chat (de la app o de Telegram). 2. Mandar la foto o contar el gasto. 3. Revisar el resumen del cambio y confirmarlo. 4. Leer los vínculos que el asistente informa; si alguno está mal, pedirle que lo deshaga. 5. Responder las preguntas de **Para revisar** (también podés pedirle «¿qué falta revisar en Finanzas?»). 6. Ver el resultado en **Finanzas**, pestaña **Resumen**; los botones de **Para revisar** llevan tu respuesta al chat. |
| **Entradas esperadas** | Ticket: comercio, fecha, productos, total y con qué cuenta o tarjeta se pagó. Resumen: la tarjeta y el archivo del resumen. Sueldo: el recibo. Cambio de moneda: reserva, cuenta, importes y si compraste o vendiste. |
| **Salidas / Resultado** | Los gastos del mes, lo pagado de tarjetas, lo que está en tarjeta a pagar, lo ahorrado, el estado de cada servicio, los productos con sus precios y las cuotas pendientes quedan actualizados. Cada vínculo automático se puede deshacer y cada duda queda en **Para revisar** hasta que la respondas. |
| **Errores comunes** | **"El movimiento pertenece a un ticket/resumen/sueldo"**: el importe, la fecha o la cuenta se corrigen desde ese documento; pedile al asistente que corrija o borre el documento. **"La reserva no alcanza para vender"**: revisá el importe o la reserva. **"La línea es de otra tarjeta, otra moneda u otro importe"**: el vínculo manual no corresponde. Una acción sin `#Confidencial` se rechaza. |

### Agenda: semana en bloques de 15 minutos y anotador del día

| Campo | Descripción |
|---|---|
| **Qué hace** | Muestra el mes y la semana elegidos, agenda tareas en bloques de 15 minutos con prioridad, lista los próximos eventos y guarda un anotador de pendientes del día. |
| **Cuándo usarlo** | Para reservar tiempo en la semana, ver de un vistazo qué días tenés tareas y anotar pendientes rápidos del día. |
| **Pasos para consumir** | 1. Abrir **Agenda** en la barra izquierda. 2. Elegir el día en el calendario o moverse con las flechas de **Semana**. 3. Seleccionar bloques en la grilla (clic o arrastre; en táctil, tocar o mantener presionado y deslizar). 4. Escribir el nombre, elegir la prioridad y tocar **Agendar**. 5. Tocar una tarea para eliminarla. 6. Anotar pendientes en **Anotador rápido** y marcarlos al terminarlos. |
| **Entradas esperadas** | Bloques de 15 minutos (hasta una semana completa por vez), nombre de la tarea (hasta 120 caracteres; vacío queda «Tarea sin título»), prioridad y notas de hasta 200 caracteres. |
| **Salidas / Resultado** | La vista se actualiza al instante y se recarga al volver a la ventana. Los días con tareas muestran un punto en el calendario y el encabezado resume los eventos próximos y las tareas pendientes. |
| **Errores comunes** | **«No se pudieron cargar los feriados»**: revisá la conexión; se reintenta solo. **Un evento de Google no aparece**: los de día completo no se traen; revisá que la cuenta tenga el permiso del calendario. **«La nota es obligatoria»**: escribí algo antes de **Agregar**. **«La biblioteca perdió su URI SAF»** (Android): volvé a seleccionar la carpeta de la biblioteca. |

### Rutina: hábitos, rachas y rueda de la vida

| Campo | Descripción |
|---|---|
| **Qué hace** | Organiza hábitos en rutinas, registra qué hiciste cada día y calcula rachas, porcentajes diarios, semanales y mensuales y el puntaje de cada categoría de la rueda de la vida. |
| **Cuándo usarlo** | Para sostener hábitos diarios o de ciertos días, revisar cómo viene la semana o el mes y equilibrar áreas de tu vida con metas por categoría. |
| **Pasos para consumir** | 1. Abrir **Rutina** en la barra izquierda. 2. Crear o renombrar rutinas en **Tus rutinas**. 3. En **Sumar a la rutina**, escribir la tarea, elegir rutina, categoría y días, y tocar **Añadir**. 4. Marcar lo hecho en **Semana actual**, eligiendo la rutina en las pestañas. 5. Ajustar las metas en **Rueda de la vida**. 6. Opcional: pedirle a la IA, por ejemplo, «marcá tomar agua y estirar como hechas hoy» y confirmar. |
| **Entradas esperadas** | Nombre de rutina (hasta 30 caracteres), nombre de tarea (hasta 60), una de las 8 categorías, días de la semana, nota opcional (hasta 80) y metas de 1 a 10. |
| **Salidas / Resultado** | El panel se actualiza al instante. Si la IA hace cambios desde el chat o Telegram, la vista abierta se recarga sola. Una tarea eliminada puede recuperarse con **Deshacer** o pidiéndoselo a la IA. |
| **Errores comunes** | **«Elegí al menos un día»**: marcá algún día o elegí **Todos los días**. **«Necesitás al menos una rutina»** o **«vaciala antes de eliminarla»**: mové o eliminá sus tareas primero. **«No se pueden marcar días futuros»**, **«no aplica el…»** o **«está pausada»**: marcá solo hoy o días anteriores, en días que correspondan a la tarea y con la tarea activa. **«Hay varias tareas llamadas…»**: indicá a la IA de qué rutina se trata. |

### Acciones IA: tareas programadas de la IA

| Campo | Descripción |
|---|---|
| **Qué hace** | Programa prompts que la IA ejecuta sola: un recordatorio, una tarea a una hora o una tarea que se repite. Muestra el estado de cada una y la línea del día. |
| **Cuándo usarlo** | Para que la IA te avise algo a una hora, arme un resumen todas las mañanas, revise el correo cada tanto o haga una tarea recurrente y te conteste por Telegram. |
| **Pasos para consumir** | 1. Abrir **Acciones IA** en la barra izquierda. 2. Tocar **Nueva acción**. 3. Elegir el tipo, escribir el nombre y el prompt y definir cuándo. 4. Revisar el resumen y las próximas ejecuciones. 5. Opcional: **Probar ahora**. 6. **Guardar acción**. Para pausarla, usar el interruptor de la tarjeta; si falló, **Reintentar**. |
| **Entradas esperadas** | Nombre (hasta 80 caracteres), prompt (hasta 4000), fecha y hora futuras (una vez) o «cada N» con unidad, días de la semana y, opcionalmente, «desde» y «hasta» (recurrente). |
| **Salidas / Resultado** | La respuesta de la IA llega por Telegram. El tablero, las métricas y la línea **Hoy** se actualizan solos al cambiar algo o al terminar una ejecución. |
| **Errores comunes** | **«La fecha y hora tienen que ser futuras»**. **«En minutos, el mínimo es 5»**. **«Elegí al menos un día»**. **«En días o semanas, «Desde» es la hora de ejecución»**. **«Telegram no está activo en este equipo…»**: activá el bot en **Configuración → Telegram** y vinculá tu chat de Owner; las acciones solo corren donde está el bot. |

### Tareas IA: trabajos largos del asistente

| Campo | Descripción |
|---|---|
| **Qué hace** | Convierte un pedido largo en una tarea que el asistente trabaja aparte: plan, pasos y revisión, con el resultado y el diagrama del flujo. |
| **Cuándo usarlo** | Para pedidos de muchos pasos (cambiar o revisar muchos documentos, investigar y escribir, ordenar mucha información) y para lo que depende de otros o del tiempo, como gestionar algo por correo hasta resolverlo. Pedíselo al asistente como siempre; él decide crear la tarea. |
| **Pasos para consumir** | 1. Pedirle el trabajo al asistente en un chat o por Telegram. 2. Confirmar la creación (o **Confirmar todos** para que no vuelva a preguntar). 3. Abrir **Tareas IA** en la barra izquierda para seguirla. 4. Responder las aprobaciones que pida. |
| **Entradas esperadas** | Las escribe el asistente: un título (hasta 120 caracteres) y el objetivo completo (hasta 12.000). |
| **Salidas / Resultado** | El paso a paso con el informe de cada paso, las revisiones, el diagrama del flujo y el resultado; mientras trabaja, el razonamiento en vivo de la fase en curso; por Telegram, el resultado al terminar. |
| **Errores comunes** | **Una espera que no avanza**: la tarea solo revisa con Notia abierta y con esa biblioteca elegida; si el correo falla, la tarjeta de espera muestra el error. **Seguir sin esperar** la hace continuar enseguida. **«El paso N no se pudo terminar»**: leé su informe y reanudala. **«La revisión pidió rehacer la tarea demasiadas veces»** (5): revisala, corregí lo que haga falta y reanudala o pedile otra revisión. **«Notia se cerró mientras la tarea trabajaba»**: tocá **Reanudar**. Una aprobación sin respuesta durante un día detiene la tarea. |

### Terminal de la PC host: comandos del asistente

| Campo | Descripción |
|---|---|
| **Qué hace** | El asistente ejecuta comandos en una terminal de la PC host y te muestra la salida en vivo. |
| **Cuándo usarlo** | Para lo que necesita la computadora: archivos fuera de la biblioteca, scripts, git, compilar, instalar, procesos, disco o red. Pedíselo al asistente como siempre. |
| **Pasos para consumir** | 1. Pedir la tarea en un chat de la app, por Telegram o como Tarea IA (solo el Owner). 2. Revisar el comando en la confirmación y tocar **Confirmar** (o **Confirmar todos**). 3. Seguir la salida en el chat o en el mensaje de Telegram. |
| **Entradas esperadas** | Las escribe el asistente: el comando (hasta 8.000 caracteres), la shell, la carpeta y el tiempo máximo (de 5 segundos a 4 horas). |
| **Salidas / Resultado** | La salida en vivo con un estado corto de la IA; en el chat, un mensaje con el comando, cómo terminó, cuánto tardó y el final de la salida; en Telegram, un mensaje por comando que se vuelve a publicar cada minuto mientras corre. |
| **Errores comunes** | **«La terminal existe solo en la PC host»**: estás en Android o en una copia sin conexión. **«se cortó al pasar su tiempo máximo»**: pedile que lo vuelva a correr con más tiempo. **Un comando que espera una respuesta** termina sin ella: pedile que use opciones no interactivas. |

### Navegador web de la PC host: el asistente usa sitios

| Campo | Descripción |
|---|---|
| **Qué hace** | El asistente maneja un navegador (Chrome o Edge) en la PC host con Playwright: abre sitios, hace clic, completa formularios, lee páginas y mira capturas. |
| **Cuándo usarlo** | Para lo que necesita un sitio de verdad: recorrerlo, buscar dentro de él, completar un formulario o ver algo de una cuenta tuya. Para buscar información general alcanza con pedirle que busque en la web. |
| **Pasos para consumir** | 1. Tener Node.js 18+ y Chrome o Edge en la PC host. 2. Pedir la tarea en un chat de la app, por Telegram o como Tarea IA (solo el Owner). 3. Si el sitio pide un captcha, iniciar sesión o un código, mirar la captura con grilla que manda el asistente y responderle qué casillas tocar, qué escribir o el código (o resolverlo en la ventana de la PC). 4. Confirmar cada clic o escritura (o **Confirmar todos**). |
| **Entradas esperadas** | Las escribe el asistente: la dirección, el elemento de la página (por su referencia y una descripción), el texto o la tecla. |
| **Salidas / Resultado** | La respuesta del asistente con lo que encontró o hizo y la dirección de la página; la ventana del navegador queda abierta en la PC host. |
| **Errores comunes** | **«El navegador necesita Node.js 18 o más nuevo»**: instalá Node.js desde nodejs.org y volvé a pedirlo. **«No se pudo instalar Playwright con npm»**: revisá la conexión o instalalo a mano con el comando que muestra el mensaje. **«No se pudo abrir Chrome ni Edge»**: instalá uno de los dos. **«El navegador existe solo en la PC host»**: estás en Android o en una copia sin conexión. |

### Scripts de soporte: herramientas en Python del asistente

| Campo | Descripción |
|---|---|
| **Qué hace** | El asistente corre como herramientas los scripts Python de `.agent/scripts`: los que escribe él para trabajos difíciles o repetitivos y los que agregás vos. |
| **Cuándo usarlo** | Para trabajos que no tienen una herramienta propia o que conviene automatizar: muchos archivos a la vez, conversiones, cálculos, análisis de datos, informes. |
| **Pasos para consumir** | 1. Tener Python 3 en la PC host. 2. Pedir el trabajo (solo el Owner); el asistente crea el script si hace falta y lo confirmás viendo el código. 3. Para uno propio, guardar `nombre.py` en `.agent/scripts` con su docstring (ejemplo en `.agent/scripts/README.md`) y pedirle que lo use. |
| **Entradas esperadas** | Los argumentos del script según sus parámetros, como JSON por la entrada estándar. |
| **Salidas / Resultado** | Lo que imprime el script (mejor en JSON), su error si falló, y la respuesta del asistente con lo que hizo. |
| **Errores comunes** | **«Los scripts de soporte necesitan Python 3»**: instalalo desde python.org con «Add python.exe to PATH». **Un script que no aparece**: revisá que el nombre sea en minúsculas con letras, números y `_` y que termine en `.py`; el asistente lista los que no pudo leer. **Falta un paquete**: instalalo con `pip` o pedile al asistente que lo instale con la terminal. **«Los scripts corren solo en la PC host»**: estás en Android o en una copia sin conexión. |

### WhatsApp: chats del asistente y autorespuesta

| Campo | Descripción |
|---|---|
| **Qué hace** | Vincula tu WhatsApp a Notia en la PC host: el asistente lee los chats que habilites, mira sus imágenes, escribe y manda archivos de la biblioteca, y Notia responde sola en los chats con autorespuesta, con el prompt de cada uno. |
| **Cuándo usarlo** | Para que el asistente te cuente qué te escribieron, conteste o mande algo por vos, o para atender solo un chat (un local, un grupo) con instrucciones fijas. |
| **Pasos para consumir** | 1. Tener Node.js 20+ en la PC host. 2. **Configuraciones → WhatsApp → Conectar** y escanear el QR desde WhatsApp → Dispositivos vinculados. 3. En cada chat, prender **Lectura**, **Escritura** o **Autorespuesta** y escribir su **Prompt**. 4. Pedirle al asistente lo que necesites (solo el Owner) y confirmar cada envío. |
| **Entradas esperadas** | Los permisos y el prompt de cada chat; en los pedidos, el chat por su nombre o número y el texto o el archivo de la biblioteca. |
| **Salidas / Resultado** | Los mensajes del chat en la respuesta del asistente, el mensaje o el archivo enviado con tu cuenta, y las autorespuestas en el chat, un minuto después del último mensaje recibido. |
| **Errores comunes** | **«WhatsApp necesita Node.js 20 o más nuevo»**: instalá Node.js desde nodejs.org y tocá **Reintentar**. **«No se pudo instalar baileys con npm»**: revisá la conexión o instalalo a mano con el comando del mensaje. **«El chat no tiene permiso de lectura/escritura para el asistente»**: prendelo en la sección. **«WhatsApp desvinculó a Notia»**: lo desvinculaste desde el teléfono (o pasó mucho tiempo sin conexión); volvé a conectar. **«WhatsApp se vincula en la PC host»**: estás en Android sin conexión a tu PC. |

### Webhook: otros programas le piden cosas a Notia

| Campo | Descripción |
|---|---|
| **Qué hace** | Un programa llama a `POST /webhook` con una acción registrada y un prompt; el asistente hace el pedido como uno tuyo y te responde por Telegram. |
| **Cuándo usarlo** | Para que un sistema (alertas de seguridad, backups, monitoreo, domótica) te avise o le pida algo al asistente sin que escribas vos. |
| **Pasos para consumir** | 1. Tener Telegram activo en la PC host. 2. **Configuraciones → Webhook**: prender **Activo**, elegir el puerto y **Guardar**. 3. **Registrar acción** y copiar su token. 4. Desde el programa, mandar `POST` a la dirección que muestra la sección con `Authorization: Bearer <token>` y `{"action": "...", "prompt": "..."}`. |
| **Entradas esperadas** | El nombre de una acción registrada y activa, su token y el prompt (hasta 8.000 caracteres). |
| **Salidas / Resultado** | `202` con el número de la llamada; el mensaje del asistente en tu Telegram y la llamada en **Últimas llamadas**. |
| **Errores comunes** | **401**: falta el token o no es el de esa acción. **404**: la acción no está registrada. **409**: la acción está pausada. **429**: más de 10 llamadas por minuto. **503**: el webhook está apagado, la biblioteca está bloqueada o Telegram no está activo en esa PC. Desde otro equipo, permití Notia en el Firewall de Windows. |

### Recetas: recetario con información nutricional

| Campo | Descripción |
|---|---|
| **Qué hace** | Guarda tus comidas en la carpeta `recipes` de la biblioteca, con foto, ingredientes, preparación, calorías, macros, vitaminas y minerales por porción. La IA revisa cada comida nueva y completa los datos que faltan. |
| **Cuándo usarlo** | Para armar tu recetario, ver qué aporta cada comida o registrar lo que cocinaste mandando una foto por Telegram. |
| **Pasos para consumir** | 1. Abrir **Recetas** en la barra izquierda. 2. Tocar **Nueva comida**. 3. Escribir al menos el nombre; opcionalmente momento, tiempo, porciones, foto, ingredientes, pasos y nutrientes. 4. **Guardar receta** y esperar la revisión de la IA. 5. Tocar una tarjeta para ver el detalle, editarla o eliminarla. Por Telegram: mandar la foto del plato con la descripción y confirmar. |
| **Entradas esperadas** | Nombre (hasta 120 caracteres, sin `/`, `\` ni `\|`), descripción (hasta 400), tiempo en minutos (hasta 1440), porciones (1 a 100), hasta 60 ingredientes y 40 pasos (uno por línea) y una foto JPG, PNG o WebP de hasta 15 MB. |
| **Salidas / Resultado** | Un archivo `.md` por receta con los datos en tablas y la foto embebida (reducida a 1024 px). El tablero se actualiza solo, también cuando la receta llega por Telegram. |
| **Errores comunes** | **«Ya tenés esta comida en el recetario»**: tocá **Ver la receta** y editala en lugar de cargarla de nuevo. **Error de la IA**: la receta no se guarda; revisá que el modelo esté configurado y probá otra vez. **«Escribí un nombre…»** u otros avisos de campo: corregí el campo marcado. |

### Salud: peso, alimentación, agua, pasos, sueño y composición corporal

| Campo | Descripción |
|---|---|
| **Qué hace** | Lleva tu peso, lo que comés cada día, el agua, los pasos, las horas de sueño y las mediciones de tu balanza. Calcula tu IMC, tu gasto diario y un plan de calorías y macros hacia tu peso objetivo, con IA o sin ella. |
| **Cuándo usarlo** | Para bajar, subir o mantener el peso, controlar calorías y macros, tomar suficiente agua o seguir tu composición corporal. |
| **Pasos para consumir** | 1. Abrir **Salud** en la barra izquierda. 2. **Configurar perfil**. 3. Fijar el **Peso objetivo** y tocar **Generar plan con IA** o **Calcular sin IA**. 4. Cargar comidas con **Agregar comida** (opcional: **Estimar con IA**), el agua y el peso de cada día. 5. Opcional: **Registrar medición** con los datos de la balanza. También se puede hacer todo desde el chat o por Telegram. |
| **Entradas esperadas** | Fecha de nacimiento (14 años o más), altura de 100 a 250 cm, peso de 20 a 400 kg, fechas que no sean futuras, comida con descripción de hasta 120 caracteres y calorías o macros (o la estimación de la IA), agua de hasta 20 litros por día. |
| **Salidas / Resultado** | El panel se actualiza al instante. Lo que registra la IA desde el chat o Telegram aparece solo en la pantalla abierta. |
| **Errores comunes** | **«La fecha no puede ser futura»**. **«El peso tiene que estar entre 20 y 400 kg»**. **«Cargá las calorías o los macros…»**: tocá **Estimar con IA** o completá algún valor. **«Fijá primero tu peso objetivo»** o **«hacen falta tu perfil y tu peso»** antes del plan. **Error de la IA**: revisá el modelo configurado, probá de nuevo o usá **Calcular sin IA**. |

### Gimnasio: rutinas, entrenamiento y equipamiento

| Campo | Descripción |
|---|---|
| **Qué hace** | Arma rutinas con los ejercicios de `Gym/exercises`, cuenta el entrenamiento y los descansos, guarda el historial y muestra el estado de cada músculo, la racha y las calorías. Filtra los ejercicios según el equipamiento que tenés. |
| **Cuándo usarlo** | Para planificar qué entrenás cada día, seguir las series en el gimnasio o en casa y ver cómo te recuperás. |
| **Pasos para consumir** | 1. Abrir **Gimnasio** en la barra izquierda. 2. **Equipamiento**: marcar lo que tenés. 3. **Ver rutinas** → **Nueva rutina** → **Editar**: elegir los días y sumar ejercicios. 4. **Entrenar** (o **Empezar** desde el panel) y marcar cada serie. 5. **Terminar** para guardar el entrenamiento. |
| **Entradas esperadas** | Nombre de hasta 120 caracteres, peso de 0 a 1000 kg, repeticiones de 0 a 10000, descanso de 0 a 10 minutos, hasta 40 ejercicios por rutina y 20 series por ejercicio. Videos MP4, WEBM o MOV de hasta 32 MB; GIF de hasta 6 MB; fotos PNG, JPG o WEBP. |
| **Salidas / Resultado** | La pantalla se actualiza al instante; las fichas y el equipamiento propio se guardan como notas en `Gym/`. |
| **Errores comunes** | **«Terminá primero el entrenamiento de …»**: hay otra rutina en curso. **«No hay ejercicios en Gym/exercises»**: la biblioteca no tiene el catálogo. **«El cuerpo se muestra masculino…»**: configurá el sexo en el perfil de **Salud**. |

### ColdPass (Credenciales Cifradas)

| Campo | Descripción |
|---|---|
| **Qué hace** | Almacena credenciales (usuarios, contraseñas, URLs, notas) en un archivo cifrado dentro de la librería activa. El cifrado ocurre localmente en el dispositivo. |
| **Cuándo usarlo** | Cuando necesitás guardar contraseñas, claves API, datos bancarios o cualquier información sensible de forma segura dentro de tu espacio de conocimiento. |
| **Pasos para consumir** | 1. En el Icon Rail, seleccionar **"ColdPass"**. 2. Ingresar la **contraseña del Owner**. La primera vez se crean la carpeta `ColdPass/` y el archivo `ColdPass.md` cifrado. 3. Agregar credenciales con **Nueva credencial** (nombre, sitio web, usuario, usuario secundario, contraseña, notas). 4. Elegir una credencial de la lista para ver su detalle, copiar la contraseña o editarla. Los cambios se cifran al guardar. |
| **Entradas esperadas** | Contraseña del Owner. Credenciales: nombre (obligatorio), sitio web, usuario, usuario secundario, contraseña y notas (opcionales). |
| **Salidas / Resultado** | Archivo `ColdPass/ColdPass.md` cifrado en el filesystem. Lista de credenciales con el estado de cada contraseña (Fuerte, Débil, Antigua) y su historial. |
| **Errores comunes** | **«La contraseña del Owner no es correcta»**: revisá mayúsculas y minúsculas. Después de varios intentos fallidos hay que esperar 30 segundos. Si se olvida la contraseña del Owner no hay recuperación: se pierden ColdPass y la configuración cifrada. **Vault anterior**: un vault creado con la passkey de antes pide esa passkey una sola vez, junto con la contraseña del Owner; después se abre solo con la del Owner. |

### Sincronización ColdPass por Bluetooth

| Campo | Descripción |
|---|---|
| **Qué hace** | Transfiere la bóveda de credenciales cifrada de un dispositivo a otro mediante Bluetooth Low Energy (BLE) con emparejamiento seguro (PIN + autenticación de aplicación). |
| **Cuándo usarlo** | Cuando querés tener la misma bóveda de credenciales en tu computadora y tu dispositivo móvil (o viceversa). |
| **Pasos para consumir** | 1. Asegurate de que ambos dispositivos tengan Bluetooth activado y estén a menos de 1 metro. 2. En el dispositivo origen, abrir ColdPass y hacer clic en **"Conectar Bluetooth"**. 3. Esperar a que detecte el dispositivo destino (nombre "ColdPass"). 4. Ingresar el PIN mostrado en el dispositivo destino. 5. Esperar la confirmación de emparejamiento. 6. Hacer clic en **"Autenticar"** para establecer canal seguro. 7. Hacer clic en **"Enviar bóveda"** para transferir las credenciales cifradas. |
| **Entradas esperadas** | PIN numérico (4-6 dígitos, mostrado en el dispositivo destino). Dispositivos con BLE compatible. En Linux, requiere BlueZ. |
| **Salidas / Resultado** | Bóveda cifrada transferida al dispositivo destino. El destino debe ingresar la misma passkey para descifrarla. |
| **Errores comunes** | **"No se encontró dispositivo"**: BLE no está activado o los dispositivos están muy lejos. Solución: acercar dispositivos y verificar Bluetooth. **"PIN incorrecto"**: solución: reintentar con el PIN correcto. **"Bluetooth no soportado"**: en Android/iOS o Windows/macOS el soporte es limitado; Linux tiene soporte completo. |

### Task Manager (Tablero, Lista y Management)

| Campo | Descripción |
|---|---|
| **Qué hace** | Organiza tareas en tableros con tres vistas: Tablero (columnas que se arrastran), Lista (grupos plegables) y Management (plan con fechas, ruta crítica, Gantt, red y tabla CPM). Cada tarea tiene prioridad, estado con cronómetro, proyecto, personas, tiempo dedicado y estimado, subtareas, dependencias, comentarios y motivo de bloqueo. |
| **Cuándo usarlo** | Para seguir el trabajo de un equipo o personal por sprints, ver quién está cargado y cuándo termina lo planificado. |
| **Pasos para consumir** | 1. En el Icon Rail, seleccionar **Task Manager**. 2. **Tablero ▾ → Nuevo tablero** (o **+** en las pestañas): nombre, color, contexto, jornada, sprints y divisiones. 3. **Personas y proyectos**: agregar quiénes trabajan en el tablero y sus proyectos. 4. **Nueva tarea** (o `N`) y completar la tarjeta: prioridad, proyecto, personas, tiempo y subtareas. 5. **Iniciar** cuando empieces; **Pausar** o **Finalizar** al terminar. 6. Mover tareas arrastrándolas o con **⋯ → Mover a**. 7. Cambiar a **Lista** o **Management** con el selector de vista. 8. En Management, editar las predecesoras desde la tabla CPM. |
| **Entradas esperadas** | Tablero: nombre, color de la paleta, contexto, jornada (inicio y fin en pasos de 30 minutos, al menos un día), sprints (1 a 8 semanas, empiezan de lunes a viernes), divisiones (1 a 12, prefijo de hasta 4 caracteres, mes de inicio). Tarea: título, prioridad, proyecto, personas del tablero, horas estimadas y dedicadas, dependencias sin ciclos. |
| **Salidas / Resultado** | Cada tarea es un archivo `.md` con frontmatter dentro de `task-manager/` (o `task-mannager/`) de la biblioteca: además de los campos de siempre guarda `inicioProgreso` mientras corre el cronómetro, `cerradaEn` al finalizarla o desestimarla y `bloqueo` con el motivo. La jornada, los sprints, las divisiones, las personas y los proyectos del tablero se guardan en `.notia-task-manager.json`; el tipo de cada columna, en el índice del tablero. |
| **Errores comunes** | **«Crearía un ciclo»**: la predecesora elegida ya espera a esta tarea. **«… no está en el tablero»**: agregá la persona en Personas y proyectos antes de asignarla. **«No se puede eliminar un ticket con subtareas»**: eliminá primero sus subtareas. **«No se puede eliminar una columna con N tareas abiertas»**: movelas, finalizalas o eliminalas antes. **«Ya no se puede deshacer»**: pasaron más de 5 segundos. |

### Pomodoro

| Campo | Descripción |
|---|---|
| **Qué hace** | Temporizador con modos Trabajo, Descanso corto y Descanso largo. Cada cierto número de pomodoros (2 a 8) toca un descanso largo. El tiempo de foco se suma a las horas dedicadas de la tarea vinculada. |
| **Cuándo usarlo** | Durante sesiones de trabajo enfocado sobre una tarea del tablero. |
| **Pasos para consumir** | 1. Abrir la pestaña **Pomodoro**. 2. En **Tarea vinculada**, elegir el tablero («Operar sobre») y **Vincular** una tarea en progreso o pendiente. 3. Elegir un preset o ajustar Trabajo, Descanso corto, Descanso largo y Ciclos. 4. **Iniciar** (o `Espacio`); **Reiniciar** (o `R`) y **Saltar** pasan a la fase siguiente. 5. Ver los pomodoros y minutos de foco del día y el mapa por hora en **Registros del día**. |
| **Entradas esperadas** | Trabajo de 5 a 90 minutos, descanso corto de 1 a 30, descanso largo de 5 a 60 y de 2 a 8 ciclos. |
| **Salidas / Resultado** | Cada fase se registra en `pomodoro.md` dentro de la carpeta de Task Manager y el foco se suma a la tarea vinculada. El temporizador lo lleva el backend, así que sigue contando si cambiás de pantalla o cerrás Notia. |
| **Errores comunes** | En el tablero publicado no se muestran los registros del día, porque mezclan todos los tableros. |

### Espacios públicos (sitios de lectura estilo GitBook)

En **Configuraciones → Publicar → Espacios públicos** (Windows) se crean sitios de solo lectura para compartir carpetas y notas en la red local, con el diseño de GitBook: índice a la izquierda, la página en el centro, «En esta página» a la derecha, anterior y siguiente al pie, búsqueda con `Ctrl + K` y tema claro u oscuro.

- **Cada espacio usa su propio puerto** y es un servidor aparte: se puede detener, cambiar o borrar uno sin tocar los demás. La URL es `http://<IP del equipo>:<puerto>/`; Notia tiene que seguir abierta. Si otro equipo no puede abrirla, permití Notia en el Firewall de Windows para redes privadas.
- **Solo se publica lo que tiene el contexto `#Publico`.** **Agregar carpeta o nota** lista las carpetas cuyo contexto de carpeta es `#Publico` y las notas con ese contexto. No vuelve a ofrecer lo que el espacio ya publica, ni las subcarpetas y notas de una carpeta ya agregada. Una carpeta publicada muestra solo sus notas `#Publico`. Si después una carpeta o nota pierde el contexto, deja de verse en el sitio y Configuraciones la marca con «Sin #Publico».
- **Cada nota de una carpeta publicada tiene la propiedad `publicado`**, una casilla en **Propiedades**. Con `true` aparece en el sitio; con `false` no aparece en el índice, la búsqueda ni los enlaces. Notia agrega `publicado: true` a las notas que no la tienen al guardar el espacio, al abrir Notia, al crear o mover una nota a la carpeta y al abrir o guardar la nota; un `false` nunca se cambia. Si se borra la propiedad, vuelve como `true`.
- **El índice se arma solo al publicar una carpeta,** como en GitBook: primero las notas de la carpeta y después un grupo por cada subcarpeta. Dentro de cada grupo, una subcarpeta es una página (su portada: la nota con el mismo nombre que la carpeta, o `README`/`index`) con el resto anidado debajo. El orden sigue las cadenas `nextPage`/`previousPage` y la fecha de creación.
- **El sitio muestra los bloques de GitBook** (avisos, pestañas, pasos, columnas, novedades, código con título, prompts, tarjetas de página, archivos, contenido incluido y condiciones). También muestra, solo para leer, lo mismo que el editor de notas: bloques de código con su lenguaje, números de línea, «Ajustar» y «Copiar»; fórmulas (también los bloques `latex`, `math` y `tex`); diagramas Mermaid; gráficos XGraph interactivos; listas de tareas con sus casillas; notas al pie; color, resaltado, subrayado y alineación del texto; bloques dentro de celdas de tabla; tablas, enlaces entre notas e imágenes. Los enlaces a notas que no están publicadas se muestran como texto. Los diagramas y gráficos usan los colores del tema del sitio y se redibujan al cambiar entre claro y oscuro (los bloques `rect` coloreados de un diagrama de secuencia pasan a un tono oscuro del mismo color en el tema oscuro, así sus textos se leen); si un diagrama no se puede dibujar, se muestra su código con el aviso correspondiente, y un gráfico XGraph con un error en su código muestra el mismo aviso que en el editor.
- **Los cambios se ven solos:** al editar una nota publicada, el sitio la muestra actualizada en pocos segundos. **Guardar y publicar** aplica los cambios de nombre, puerto, contenido y estado.

### Publicar Task Manager en la red local

El chat de IA del panel publicado envía la consulta al host y recibe el resultado por streaming; la app host ejecuta el motor global de Notia con sus límites, permisos, confirmaciones y herramientas del alcance publicado. El navegador no contacta Ollama ni recibe la URL o la API key.

Desde **Configuraciones → Publicar**, el anfitrión elige los tableros y comparte la URL HTTPS `/task-manager`. El login valida el usuario y la contraseña de la cuenta de la biblioteca; no existe una contraseña adicional de tablero ni una aprobación de dispositivos. Una vez autenticado, puede editar tareas desde la misma interfaz: las mutaciones viajan por WebSocket seguro y los cambios confirmados aparecen en el anfitrión y en los demás navegadores autorizados.

La URL publicada solicita un nombre de usuario y la contraseña personal de esa cuenta. Rust autentica contra `library_users` y crea una cookie `HttpOnly` asociada al `user_id`; la contraseña nunca se guarda en texto plano ni se recuerda en el navegador. Cambiar la contraseña, quitar el usuario, cambiar sus contextos o cerrar/republicar la publicación invalida el acceso correspondiente.

Los ajustes de sincronización requieren reiniciar el proceso de escritorio con el código actualizado y recargar las páginas publicadas. La URL recibe en el arranque la ubicación lógica de la carpeta de tareas y la reutiliza durante toda la sesión, por lo que mover varios tickets seguidos no genera consultas repetidas de detección de carpetas ni agota el límite de solicitudes. Host e invitados esperan su turno ante acciones simultáneas, y un lote conserva la misma identidad desde el inicio hasta su confirmación; se pueden mover repetidamente los mismos tickets, crear, editar detalle/Markdown, comentar y cambiar estado sin dejar bloqueado el siguiente cambio. Las operaciones sobre tickets diferentes no chocan solo porque avanzó la revisión global; si dos personas editan el mismo archivo desde versiones incompatibles, se informa el conflicto en lugar de sobrescribirlo. Si la sesión realmente vence o el host vuelve a publicar, el navegador regresa al ingreso en vez de continuar acumulando errores `401`/`429`. La validación visual multiusuario sigue pendiente y no debe darse por certificada únicamente por las pruebas automatizadas.

La capacidad está configurada entre 1 y 64 sesiones autenticadas y conexiones WebSocket simultáneas (64 por defecto). Si se alcanza el límite, el nuevo ingreso recibe un mensaje para reintentar; las sesiones existentes continúan activas. Si la red se corta, el navegador reconecta y recupera eventos pendientes; si ya no están disponibles, descarga un snapshot nuevo. Una edición Markdown que parta de una revisión vieja muestra conflicto y conserva el texto escrito, sin sobrescribir el cambio remoto. Revocar el acceso de un usuario, vencer su sesión o detener/republicar la publicación cierra sus conexiones y finaliza sus streams.

Para integraciones o diagnóstico, el flujo HTTP usa `POST /task-manager/login` con `{ "username", "userPassword" }` y `GET /task-manager/bootstrap` con la cookie de sesión. No hay endpoint de aprobación de dispositivos ni campo `boardPassword`; el identificador de dispositivo no autoriza el login. Las mutaciones no deben enviarse a `/invoke`: el canal colaborativo es `wss://<host>/task-manager/ws`, con mensajes JSON como:

```json
{
  "type": "mutate",
  "protocolVersion": 1,
  "messageId": "mensaje-opaco",
  "operationId": "operacion-opaca",
  "baseRevision": 12,
  "command": "task_manager_board_execute",
  "args": { "payload": { "intent": { "kind": "change-state", "taskPath": "task-mannager/board/task.md", "state": "En progreso" } } }
}
```

El servidor responde con un `ack` y distribuye un evento `changed` con `publicationEpoch`, `sequence`, `revision` y `messageId`. Las rutas locales, credenciales, contenido no autorizado y preferencias privadas no forman parte del protocolo.

`GET /task-manager/status` requiere la misma cookie autenticada y devuelve solo métricas agregadas del host para diagnóstico; nunca expone rutas locales, credenciales ni contenido de tareas.

### Host y Cliente: la misma biblioteca en varios dispositivos

En **Configuraciones → General → Modo de ejecución** elegís cómo corre Notia en cada equipo:

- **Host**: la computadora (Windows, Linux o macOS) que guarda la biblioteca. Los otros dispositivos se sincronizan con ella por el puerto 52480, que se ve con **Escuchando**. La primera vez, Windows puede pedir permiso de red para Notia: aceptalo para que los otros equipos lleguen. En Android, **Host** significa que el teléfono usa su propia biblioteca, sin sincronizarla.
- **Cliente**: un teléfono, una tablet u otra computadora que guarda una copia de la biblioteca del host y trabaja siempre sobre ella. Escribí la dirección del host (por ejemplo `192.168.0.10:52480`) y tocá **Guardar**; la ventana se recarga. En Android primero elegís una carpeta vacía con **Elegir carpeta**: ahí queda la copia.

**Cómo funciona el cliente:**

- **La primera vez** iniciás sesión con el usuario **Owner** del host y Notia copia la biblioteca entera, con la base de datos y lo escrito a mano sobre las notas. La pantalla **Copiando la biblioteca** muestra cuántos archivos van. En un celular puede tardar unos minutos; sigue aunque pongas Notia en segundo plano y, si se corta, sigue donde quedó.
- **Después, Notia abre al instante sobre la copia**, haya conexión o no. Ya no aparece «Conectando con el host…». Iniciás sesión con el Owner como en cualquier biblioteca; con **Recordar sesión** entra solo.
- **Todo funciona sin conexión**: notas, archivos y los datos de Finanzas, Agenda, Rutina, Salud, Gimnasio, Recetas y ColdPass. El chat con IA corre en el dispositivo con el proveedor configurado (por ejemplo, Ollama en la nube) y las herramientas de la biblioteca. El dictado y **Meeting** graban y transcriben en el dispositivo.
- **Cuando el host responde, se sincroniza solo** cada pocos segundos, en segundo plano: sube lo que cambiaste y baja lo que cambió en el host, tanto los archivos como los registros de la base. Si un mismo archivo o registro cambió en los dos lados, queda el cambio más reciente (cuenta el reloj de cada equipo). Ya no hay edición en conjunto: si dos dispositivos editan la misma nota entre dos sincronizaciones, queda la versión más nueva.
- **Estado:** en Android, una notificación fija dice si está buscando el host, sincronizado, sincronizando (con cuántos archivos van), sin conexión o con un error. La primera vez, Android pide permiso para mostrar notificaciones. En las computadoras se ve en **Configuraciones → General → Sincronización**, con el botón **Sincronizar ahora**. **Probar conexión** muestra **Conectado** o **No responde**.
- **Contraseña:** si la cambiás en otro equipo, iniciá sesión en el cliente con la nueva: Notia la confirma con el host y trae la copia al día. Cambiarla desde un cliente necesita conexión con el host.

**Telegram y los servicios de la biblioteca** (Telegram, WhatsApp, el webhook, el agente autónomo, las Acciones y Tareas IA, la sincronización con Google Calendar y los backups) corren en una sola computadora: la de **más procesamiento** entre las que se ven (más núcleos y después más memoria). Configuraciones → General lo dice en **Servicios de la biblioteca**: **En esta computadora** o **En otra computadora**. Si la computadora que los corre se apaga, los toma la otra; Android nunca los corre. Telegram, WhatsApp y el webhook se configuran en la computadora en modo Host. La terminal, el navegador y los scripts del asistente funcionan en cualquier computadora, también en un cliente, pero no en Android.

### Servidor Notia sin ventana (headless)

El mismo `notia.exe` puede quedar corriendo como servidor, sin abrir la ventana, para que otros equipos de la red usen Notia. En Linux se compila solo el servidor. Usa las mismas bibliotecas y datos que la aplicación, así que no pueden estar abiertas las dos a la vez sobre la misma carpeta de datos. Si se intenta, Notia avisa que ya está en uso.

1. Definí la contraseña del dueño: `notia --headless --set-owner-password`.
2. Si hace falta, agregá bibliotecas de ese equipo: `notia --headless --add-library "D:\Notas"`.
3. Arrancá el servidor: `notia --headless`. Muestra la dirección `https://<ip>:52480/`. Con `--static-dir` sirve también la interfaz web, y `--bind` cambia la dirección o el puerto.

Desde otro equipo o un teléfono, abrí esa dirección en el navegador. El certificado es autofirmado, por eso el navegador pide confirmarlo la primera vez. Después de ingresar la contraseña del dueño se usa la misma interfaz de Notia: bibliotecas, notas, chat, Task Manager, Finanzas y el resto, con los cambios en vivo. El dictado del chat graba con el micrófono del dispositivo desde el que usás Notia y el texto aparece al finalizar. Lo que depende del equipo servidor no se ofrece en el navegador: Meeting, el Bluetooth de ColdPass, importar un vault CSV, elegir la carpeta de backups y agregar librerías con el selector. Para que el servidor sirva la interfaz, indicá `--static-dir` con la carpeta `dist` compilada o dejala junto al ejecutable.

Si el navegador pierde la conexión, se reconecta solo y recibe los cambios que ocurrieron mientras tanto. Si pasó demasiado tiempo o el servidor se reinició, aparece un aviso arriba con el botón **Recargar** para volver a leer todo.

En Linux, el servidor se compila desde `src-tauri` con `cargo build --release --no-default-features`. Allí no hay dictado ni Bluetooth.

### Búsqueda de Archivos

| Campo | Descripción |
|---|---|
| **Qué hace** | Busca archivos por nombre dentro de la librería activa, mostrando resultados en tiempo real mientras escribís. |
| **Cuándo usarlo** | Cuando necesitás encontrar rápidamente una nota o documento sin navegar manualmente por el árbol. |
| **Pasos para consumir** | 1. En el panel del Explorador (izquierda), hacer clic en la barra de búsqueda. 2. Escribir el nombre o parte del nombre del archivo. 3. Los resultados se filtran automáticamente en el árbol. 4. Hacer clic en un resultado para abrirlo. |
| **Entradas esperadas** | Query de búsqueda (string, mínimo 1 carácter). Búsqueda case-insensitive y sin acentos. |
| **Salidas / Resultado** | Lista de archivos cuyo nombre coincide con el query. Si no hay coincidencias, el árbol muestra estado vacío. |
| **Errores comunes** | **"No hay resultados"**: el archivo no existe o está en otra librería. Solución: verificar la librería activa o el nombre del archivo. |

---

## 📖 Guía de Uso

### Inicio Rápido

1. **Abrir la aplicación**: al iniciar por primera vez, Notia no tiene librerías configuradas.
2. **Agregar una librería**:
   - En el panel izquierdo, abajo, tocá el selector de librería y elegí **"Administrar librerías"**.
   - Seleccioná **"Agregar librería"** y elegí una carpeta de tu filesystem (escritorio) o concedé permisos de carpeta (Android).
   - La carpeta seleccionada se indexará y aparecerá en el Explorador.
3. **Crear contenido**:
   - Desde el encabezado del Explorador: **Nueva nota**, **Nuevo diagrama** o **Nueva carpeta** (también `Ctrl + N` para una nota nueva).
   - Sin ninguna nota abierta, la pantalla central ofrece **Nueva nota**, **Nuevo chat** (abre la vista Chat IA en un chat nuevo) e **Ir a archivo**.
   - O desde el **menú contextual** (clic derecho) en cualquier carpeta del Explorador.
4. **Abrir archivos**: hacé clic en cualquier archivo del árbol de archivos. Se abrirá en una pestaña.
5. **Navegar entre vistas**: el **Icon Rail** (barra vertical izquierda) permite cambiar entre:
   - **Explorador**: muestra u oculta el panel de archivos.
   - **Vista de grafo**, **Chat** y **Task Manager**.
   - **ColdPass**, **Transcribir meeting** y **Finanzas**.
   - **Calendario**, **Agenda** y **Rutina**.
   - Abajo, **Ayuda** y **Configuración**. En escritorio, cada ícono muestra su nombre al pasar el mouse; el módulo abierto queda marcado en teal.
6. **Buscar un archivo**: escribí en **Buscar archivos**, arriba del árbol, o usá `Ctrl + O`.
7. **Cerrar pestañas**: clic en la "X" de la pestaña, o atajo `Ctrl + W`.
8. **Paneles**: los botones a la derecha de las pestañas muestran u ocultan el Explorador y el Asistente, y cambian el tema. En un teléfono, el Explorador se abre sobre el contenido.

### Uso de Wikilinks

1. En cualquier nota Markdown, escribí `[[Nombre de otra nota]]`.
2. Notia resaltará el wikilink y mostrará sugerencias mientras escribís.
3. Hacé clic en el wikilink para abrir la nota destino en una nueva pestaña.
4. Si la nota destino no existe, Notia te ofrecerá crearla.

### Usar Enlaces Secuenciales (Page Links)

1. Abrí una nota Markdown desde el Explorador.
2. En el panel **Propiedades** (arriba del texto), buscá la propiedad `nextPage`. Si no existe, se crea automáticamente al abrir la nota por primera vez.
3. Tocá **Vincular nota…** (o el lápiz, si ya apunta a una nota).
4. Escribí parte del nombre: aparece la lista de notas de la librería, cada una con su carpeta.
5. Elegí la nota (o presioná **Enter** para la resaltada). Si todavía no existe, tocá **Crear nota «…»**: Notia la crea junto a la nota actual y la enlaza.
6. Notia guardará automáticamente:
   - La nota actual con `nextPage: [[nombre-del-siguiente-archivo]]`
   - La nota destino con `previousPage: [[nombre-de-la-nota-actual]]`
7. En el **Explorador**, ambas notas aparecerán conectadas visualmente como un bloque secuencial.
8. Para **romper un link**, tocá el lápiz de `nextPage` (o `previousPage`) y elegí **Quitar enlace**. Notia limpiará el vínculo opuesto automáticamente.
9. Para **cambiar el destino**, editá `nextPage` a un nuevo archivo. Notia limpiará el vínculo en el destino anterior y lo creará en el nuevo.
10. **Nota**: No se permiten ciclos (A → B → C → A). Si intentás crear un ciclo, Notia mostrará un error.

### Configurar el Chat con IA

1. Asegurate de tener **Ollama** instalado y corriendo localmente en tu máquina (o accesible en red local).
2. En Notia, abrí **Settings** (⚙️) → **IA**.
3. Ingresá la URL de Ollama (por defecto: `http://localhost:11434`).
4. Opcional: ingresá una API Key si usás Ollama Cloud o un servicio con autenticación.
5. Hacé clic en **"Verificar conexión"** para confirmar que Notia puede comunicarse con Ollama.
6. Seleccioná un modelo de la lista de modelos disponibles. Para enviar imágenes, elegí uno con capacidad de visión.

### Usar ColdPass

1. En el **Icon Rail**, seleccioná **ColdPass**.
2. Ingresá la **contraseña del Owner**. La primera vez se crean una carpeta `ColdPass/` y un archivo `ColdPass.md` cifrado en tu librería activa. Si tu vault es de antes, también te pide su passkey anterior, una sola vez. Si activaste la huella en ese dispositivo Android, se pide la huella: podés apoyar el dedo o usar la contraseña.
3. Agregá, editá o eliminá credenciales. Cada cambio se cifra automáticamente al guardar. Para eliminar te vuelve a pedir la contraseña del Owner.
4. Usá los filtros **Débiles** y **Antiguas** para encontrar las contraseñas que conviene cambiar, y **Generar nueva** para reemplazarlas.
5. Para sincronizar con otro dispositivo:
   - Asegurate de que ambos dispositivos tengan Bluetooth activado.
   - En el dispositivo origen, iniciá la conexión Bluetooth desde ColdPass.
   - Seguí las instrucciones de emparejamiento (PIN y autenticación).
   - Transferí la bóveda cifrada de forma segura.

### Usar el Task Manager y Pomodoro

1. En el **Icon Rail**, seleccioná **Task Manager**.
2. Creá un **tablero** con **+** en las pestañas: elegí nombre, color, contexto, jornada, sprints y divisiones del año.
3. En **Personas y proyectos** agregá a las personas que trabajan en el tablero y sus proyectos.
4. Tocá **Nueva tarea** (o la tecla `N`) y completá la tarjeta: prioridad, proyecto, personas, tiempo estimado y subtareas.
5. **Iniciar** pone en marcha el cronómetro de la tarea; **Pausar** o **Finalizar** suman el tiempo a lo dedicado. Si te equivocaste al finalizar, desestimar o eliminar, tocá **Deshacer** en el aviso.
6. Cambiá entre **Tablero**, **Lista** y **Management**. En Management ves cuándo termina cada tarea, quién está cargado y la ruta crítica; editá las predecesoras desde la tabla.
   - Tocá el título de una tarea para abrir su ticket, o **⋯ → Editar nota** para editarla con el mismo editor de las notas.
7. Las tareas finalizadas y desestimadas pasan a **Completadas** y **Canceladas**; desde ahí podés recuperarlas o eliminarlas.
8. Para usar el **Pomodoro**, abrí su pestaña, vinculá una tarea e iniciá: el foco se suma a sus horas dedicadas.

### Cambiar el Tema y Preferencias

1. Abrí **Settings** (⚙️) desde la barra de título o el menú.
2. En **Apariencia**, seleccioná **Claro** u **Oscuro**.
3. En **Explorador**, ajustá el **intervalo de refresco** (útil en Android para detectar cambios externos).
4. En **InkMath**, configurá el intervalo de inactividad previo al reconocimiento OCR.

---

## 🛠️ Requisitos de Sistema

### Para ejecutar la aplicación

- **Sistema operativo**: Windows 10+, macOS 11+, Linux (kernel 5.x+), Android 10+.
- **Para IA local**: instancia de **Ollama** corriendo localmente o accesible en red (opcional).
- **Para ColdPass Bluetooth**: adaptador Bluetooth Low Energy (BLE) compatible (Linux requiere BlueZ; Windows y macOS tienen soporte limitado actualmente).

### Para desarrollo

- **Node.js**: 20 o superior.
- **npm**: 10 o superior.
- **Rust Toolchain**: `rustup`, `cargo`, `rustc` (edición 2021).
- **Git**: para control de versiones.

### Dependencias del sistema (Linux)

Para compilar y ejecutar Notia en Linux, se requieren los siguientes paquetes del sistema:

```bash
# Ubuntu / Debian
sudo apt install libwebkit2gtk-4.1-dev \
    build-essential \
    curl \
    wget \
    libssl-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libdbus-1-dev \
    libbluetooth-dev

# Fedora
sudo dnf install webkit2gtk4.1-devel \
    gcc \
    gcc-c++ \
    make \
    curl \
    wget \
    openssl-devel \
    gtk3-devel \
    libayatana-appindicator3-devel \
    librsvg2-devel \
    dbus-devel \
    bluez-devel
```

📖 **Guía oficial de prerequisitos de Tauri**:  
https://tauri.app/start/prerequisites/

---

## 🚀 Instalación y Desarrollo

### Clonar el repositorio

```bash
git clone <repository-url>
cd notia
```

### Instalar dependencias

```bash
# Frontend (Node.js)
npm ci

# Backend (Rust) — se descarga automáticamente con cargo durante el build
```

`npm ci` instala exactamente las versiones de `package-lock.json`, incluido el override de `esbuild` usado para estabilizar el build en Windows. `npm install` también es válido para instalaciones de desarrollo que necesiten actualizar el lockfile.

### Modo desarrollo

Los launchers de Tauri generan automáticamente el build multipágina de desarrollo antes de iniciar la app, sin minificar JavaScript. En Windows, esbuild `0.27.3` podía terminar con `-1073741819` (`STATUS_ACCESS_VIOLATION`) y Vite mostraba `The service was stopped` al generar esos assets. El proyecto fija `esbuild` en `0.27.7` mediante `package.json` y `package-lock.json`; esto también prepara `public-task-manager.html`, que el servidor HTTPS local necesita para entregar la pantalla posterior al login de la URL publicada de Task Manager. El `npm run build` normal continúa siendo el build optimizado para producción.

```bash
# Solo frontend web (Vite, puerto 1420)
npm run dev

# App desktop completa (Linux, auto-detecta Wayland/X11)
npm run dev:tauri

# App desktop completa en Windows
npm run dev:tauri:windows

# Forzar backend Wayland
NOTIA_TAURI_BACKEND=wayland npm run dev:tauri:wayland

# Forzar backend X11
NOTIA_TAURI_BACKEND=x11 npm run dev:tauri:x11

# Wayland con fallback a X11
NOTIA_TAURI_BACKEND=wayland NOTIA_TAURI_FALLBACK_X11=1 npm run dev:tauri:wayland:fallback
```

En Windows, si el puerto 1420 ya está ocupado por una instancia de Vite iniciada desde este mismo repositorio, el comando la reutiliza. Si pertenece a otra aplicación o proyecto, informa el proceso que debe cerrarse y no inicia Tauri contra un servidor incorrecto.

El launcher de Windows reintenta una vez el build de desarrollo si esbuild se detiene. Si ambos intentos fallan, revisá procesos Node/Vite duplicados y el antivirus antes de volver a ejecutar el comando.

### Desarrollo para Android

```bash
# Desarrollo en dispositivo Android (auto-detecta NDK y dispositivo adb)
npm run dev:android

# Build debug APK
npm run build:android:debug

# Build release AAB (Android App Bundle para Play Store)
npm run build:android:aab

# Instalar release en dispositivo conectado
npm run install:android:release
```

Cada build copia a `builds/android/` solo el APK que acaba de generar. Si Gradle no produce uno, el comando falla en vez de instalar una versión anterior.

---

## ⚙️ Configuración de Entorno y Preferencias

Notia guarda las preferencias en su backend: las del dispositivo en los datos de la aplicación y las de IA, Telegram y contextos en la configuración de cada biblioteca. En el navegador o WebView solo quedan preferencias visuales, como el tema, el ancho de paneles o las carpetas expandidas. Las siguientes configuraciones están disponibles:

### Apariencia
- **Tema**: claro u oscuro. Persiste entre sesiones.

### IA (Ollama)
- **URL de Ollama**: dirección del servidor local de Ollama (default: `http://localhost:11434`).
- **API Key**: opcional, para servicios que requieren autenticación.
- **Modelo seleccionado**: elegí entre los modelos multimodales disponibles detectados automáticamente.
- **Feedback del agente**: estándar, mínimo, detallado o desactivado; el plan y el resumen de enfoque se pueden ocultar.

Las ediciones documentales se proponen con diff por hunks, revisión exacta, confirmación y undo por `operationId`. La memoria activa vive en `.agent/memory/memory.md`; los chats persistentes del Owner y Telegram vinculado al Owner pueden cargarla y guardarla, mientras Meeting y Telegram vinculado a otro usuario no la cargan ni la guardan. La búsqueda web de Ollama solo admite consultas públicas sanitizadas y no envía claves, datos personales ni contenido del workspace.

### Explorador de archivos
- **Intervalo de refresco**: en Android, podés configurar un intervalo en milisegundos para que el árbol de archivos se refresque periódicamente (default: deshabilitado).

### InkMath
- **Espera de OCR**: intervalo de inactividad antes de enviar a Ollama los trazos de una fórmula dibujada.

### Logging
- Notia registra solamente errores, salvo una traza diagnóstica acotada para imágenes procesadas por Telegram/Ollama. Esa traza muestra descarga, rondas de IA, nombres de tools, persistencia, respuesta y tiempos sin incluir la imagen, el prompt, credenciales ni argumentos financieros.

---

## ❓ FAQ y Troubleshooting

### No veo mis archivos en Android
- Verificá que hayas concedido los permisos de acceso a la carpeta mediante el selector del sistema (SAF).
- Asegurate de que la URI de la carpeta (Android Tree URI) esté correctamente asociada a la librería.
- Si la carpeta fue modificada externamente, usá el botón de refresco manual o configurá un intervalo de refresco automático en Settings.

### Notia se trababa en Android (escribir notas, Gimnasio)
- Desde la versión del 2026-10-02, la lectura y escritura de archivos y de la base de datos ya no ocupan el hilo que maneja el toque y la pantalla: la app sigue respondiendo mientras guarda.
- Al escribir una nota, Notia procesa el texto en cada pausa y no en cada tecla; guardar una nota ya no relee todas las demás.
- La base de la biblioteca guardaba un registro de cada pedido al asistente y no lo borraba nunca (en una biblioteca llegó a 72 MB). Ahora conserva, por usuario, los últimos 48 pedidos de la última semana, y la base se compacta sola. Lo que se deshace o se retoma de un pedido del asistente funciona dentro de ese rango.
- **Recetas** y **Gimnasio** aparecían vacíos en Android aunque la carpeta tuviera sus archivos: ya se leen bien.

### Al abrir el chat IA la pantalla se queda en blanco o se dejan de ver los mensajes
- Si la app se volvía blanca, ese problema fue corregido. Si persistiera, reiniciá la app y verificá que no haya quedado cacheado el bundle anterior (`npm run dev:tauri` o volvé a instalar la app en Android).
- Si los mensajes del asistente aparecen cortados, asegurate de usar la última versión del código: el hilo de chat ahora renderiza cada mensaje con su altura real, sin forzar un tamaño fijo que recorte contenido largo.
- Reportá cualquier traza adicional que aparezca en la consola de desarrollo.

### La IA no responde o da error de conexión
- Verificá que **Ollama** esté corriendo localmente (`ollama serve` en terminal).
- Confirmá que la URL en **Settings → IA** coincida con la dirección de Ollama (generalmente `http://localhost:11434`).
- Si estás en Android, asegurate de que el dispositivo tenga acceso de red a la instancia de Ollama (no funciona offline a menos que Ollama corra en el mismo dispositivo).

### Windows muestra `The service was stopped` durante el build de desarrollo
- Instalá las dependencias desde el lockfile con `npm ci` y comprobá la versión efectiva: `node -e "require('esbuild').version"` debe devolver `0.27.7`.
- Ejecutá `npm run build -- --minify=false` para validar el build multipágina de desarrollo. `npm run dev:tauri:windows` ya lo ejecuta sin minificación y reintenta una vez si el proceso nativo de esbuild termina con error.
- Si el segundo intento también falla, cerrá instancias duplicadas de Node/Vite y revisá el antivirus. El fallo observado correspondía a `STATUS_ACCESS_VIOLATION` de esbuild `0.27.3`, no a la minificación de la aplicación.

### El cliente no se conecta al host
- Revisá que el host esté en modo **Host** y muestre **Escuchando** en Configuraciones → General. Si dice **Sin escuchar**, otro programa usa el puerto.
- Comprobá que los dos equipos estén en la misma red y que el firewall del host permita a Notia.
- En la pantalla «Sin conexión con el host» podés reintentar, cambiar la dirección o volver a usar el equipo como host.

### El cliente dice «El certificado del host cambió»
- El cliente recuerda el certificado del host desde la primera conexión. Si reinstalaste Notia en el host, editá su dirección en **Configuraciones → General** y tocá **Guardar** para confiar en el nuevo.

### «Notia ya está en uso» al abrir la app o el servidor
- La app y el servidor sin ventana (`notia --headless`) comparten la misma carpeta de datos, y solo uno puede usarla a la vez. Cerrá el otro proceso (o la otra ventana de Notia) y volvé a abrir.

### El navegador avisa que la conexión con el servidor Notia no es segura
- El servidor usa un certificado propio (autofirmado). Confirmá la excepción una vez para esa dirección. Si cambiaste de red y la IP es otra, puede pedirlo de nuevo.

### El dictado no funciona desde el navegador
- El navegador solo permite el micrófono en páginas HTTPS: entrá con la dirección `https://…` que muestra el servidor y aceptá el permiso de micrófono.
- La transcripción la hace el servidor: necesita el reconocimiento de voz activado en **Configuraciones → Voz** y un servidor Windows. En Linux no está disponible.

### El tema no se guarda entre sesiones
- Verificá que tu navegador o WebView no esté en modo privado/incógnito (bloquea localStorage).
- Si estás en Android, asegurate de que la app tenga permisos de almacenamiento local.

### Wayland no funciona en Linux o la ventana se ve mal
- Probá forzar el backend X11: `NOTIA_TAURI_BACKEND=x11 npm run dev:tauri:x11`.
- O usá el modo fallback a X11: `NOTIA_TAURI_BACKEND=wayland NOTIA_TAURI_FALLBACK_X11=1 npm run dev:tauri:wayland:fallback`.

### ColdPass no sincroniza por Bluetooth
- Asegurate de que el Bluetooth esté activado en ambos dispositivos.
- En Linux, verificá que el servicio BlueZ esté corriendo y que tu adaptador soporte BLE.
- Confirmá que ingresaste el PIN correcto durante el emparejamiento.
- Mantené los dispositivos a menos de 1 metro de distancia durante la sincronización.

### La aplicación se siente lenta con bibliotecas muy grandes
- El Explorador usa virtualización (`useVirtualList`) para renderizar solo los nodos visibles; árboles de miles de archivos deberían mantenerse fluidos.
- Los diagramas Mermaid embebidos renderizan de forma **lazy** (solo cuando entran al viewport) y cancelan renders previos al cambiar de archivo.
- Notia aplica memoización selectiva (`React.memo`, `useMemo`, `useCallback`) y selectores de acciones (`useNotiaAction`) para reducir re-renders del panel izquierdo, el workspace y el panel derecho.
- En Android, Notia aplaza la carga de vistas pesadas (Graph, Chat, Task Manager) un par de frames para mantener la UI responsiva.
- En Android, Notia ajusta automáticamente la caché de renders Mermaid a 10 entradas / 2 MB para reducir consumo de memoria, mientras que en desktop conserva 20 / 5 MB.
- Al cambiar de biblioteca o cerrar todos los documentos, Notia invalida la caché de renders Mermaid para liberar SVGs de la librería anterior.
- Los componentes pesados (`MarkdownView`, `MermaidView`, `GraphView`) limpian sus recursos al desmontar: destruyen editores, remueven canvas, cancelan timeouts y limpian listeners globales.
- Las vistas pesadas usan selectores Redux memoizados (`selectTheme`, `selectMermaidViewerState`, `selectActiveLibraryPath`) en lugar de funciones inline, reduciendo re-renders en cadena.
- Las vistas más pesadas (`MarkdownView`, `MermaidView`, `ChatWorkspaceView`, `GraphView` y `TaskManagerApp`) se cargan bajo demanda mediante `React.lazy`, con `Suspense` y fallback mínimo, así el bundle inicial no incluye el editor Milkdown/Crepe, Monaco, Mermaid, react-force-graph-2d, Cytoscape ni MUI.
- En escritorio, Notia precarga esas vistas de forma inteligente durante los momentos de inactividad (`requestIdleCallback`) para que la primera apertura de archivo sea instantánea; en Android la precarga se omite por defecto para ahorrar memoria y datos.
- `vite.config.ts` agrupa dependencias grandes en chunks separados (`vendor-milkdown`, `vendor-mermaid`, `vendor-iconify-packs`, `vendor-mui`, `vendor-cytoscape`, `vendor-lucide`, etc.), manteniendo el bundle inicial en ~460 KB gzip.
- Los icon packs de Mermaid (`@iconify-json/*`) y las librerías de exportación PDF (`jspdf`, `html2canvas`) se cargan dinámicamente solo cuando se abre el menú de iconos o se exporta un PDF, respectivamente.
- El backend Rust se compila con perfil de release optimizado (`lto = true`, `codegen-units = 1`, `strip = true`, `panic = "abort"`) para reducir tamaño de binario y mejorar rendimiento en Android.
- La lectura del árbol, la búsqueda y la lectura masiva de Markdown corren en el backend en un pool de hilos (`spawn_blocking`), sin bloquear la interfaz. Lo mismo vale en el servidor sin ventana.
- En Android, Notia cachea resoluciones SAF en una LRU Rust-side de 500 entradas y throttlea refrescos de cache a 200 ms, reduciendo llamadas JNI.
- Los eventos de cambio en el árbol de archivos (`notia-library-tree-changed`) se agrupan (batch) 160 ms para evitar refrescos en cascada durante guardados o pegados múltiples.
- Considerá dividir tu conocimiento en múltiples librerías más pequeñas si un solo árbol supera varios miles de archivos.

---

## 📝 Licencia

Copyright © 2026 Gabriel. Todos los derechos reservados.

---

**Notia** — Tu espacio de conocimiento, organizado.

## Cambios del motor de la aplicación

Notia ejecuta ahora toda la lógica de la aplicación en su motor nativo; la interfaz solo muestra resultados y envía tus acciones. En el uso diario esto cambia lo siguiente:

- **Telegram** funciona aunque la ventana de Notia esté cerrada, y también en Android. Si le enviás al bot un PDF escaneado (sin texto), te pide fotos de las páginas.
- **Notas editadas por el agente**: si el agente modifica una nota que tenés abierta, Notia la recarga. Si además tenías cambios sin guardar, al guardar te avisa del conflicto en lugar de pisar la versión del agente.
- **Confirmaciones**: ya no existe la opción de aplicar automáticamente los cambios de bajo riesgo; todo cambio del agente pide confirmación. En los planes del agente se quitó **Reintentar paso fallido**; **Continuar TO-DO** y **Cancelar** siguen disponibles.
- **Preferencias por dispositivo**: el prompt elegido del agente, las opciones de voz y la publicación de Task Manager se guardan en cada dispositivo, no en la biblioteca. Las elecciones anteriores se conservan.
- **Publicación de Task Manager**: en Windows se vuelve a publicar sola al abrir Notia, con tema oscuro.
- **Task Manager**: un ticket se mueve a Completadas o Canceladas solo cuando cambiás su estado; una subtarea finalizada que guardás junto a su tarea padre queda donde está. Los comentarios usan el formato con fecha, hora y autor descrito en **Task Manager**.
- **ColdPass por Bluetooth**: la passkey de la sesión ya no queda en la interfaz. La sincronización sigue disponible solo en Linux.

## Roles, usuarios y acceso

En Telegram, el agente recibe instrucciones específicas para responder sin Markdown escapado y usando únicamente el subconjunto HTML admitido por Telegram.

El agente de Telegram genera directamente la respuesta en el formato del canal. Si Telegram rechaza una entidad mal formada, Notia reintenta el envío como texto plano para no perder la respuesta.

En **Configuraciones → Roles** se administran los roles de la biblioteca activa; cada biblioteca comienza con `Owner`, `Family` y `Guest`. En **Configuraciones → Usuarios** se pueden crear usuarios, asignarles un rol, cambiar su nombre o contraseña y eliminar usuarios que no sean `Owner`. Las acciones de cada usuario se muestran como iconos compactos con tooltip y soporte de teclado. Estos datos viven en el SQLite de cada biblioteca y no se mezclan al cambiarla. Las contraseñas solo se almacenan como hashes PBKDF2 y los usuarios sin contraseña se muestran como **Sin contraseña**.

La vinculación de Telegram se realiza en un chat privado escribiendo `/start`, el nombre de usuario de Notia y la contraseña correspondiente. Una cuenta no vinculada queda bloqueada fuera de ese flujo. El Task Manager publicado usa el mismo usuario y contraseña de la biblioteca; ya no requiere una contraseña adicional del tablero.

## Acceso actualizado a publicación y Telegram

La publicación de Task Manager usa únicamente las cuentas de la biblioteca: no existe una contraseña adicional del tablero, aprobación de dispositivos ni almacenamiento local de credenciales. El login muestra estados accesibles de carga, error, sesión inválida y reintento; las sesiones autenticadas quedan asociadas al `user_id` de SQLite.

El enlace de Telegram se resuelve por chat privado contra SQLite. El flujo limita los intentos, aplica un enfriamiento temporal al alcanzar el límite y mantiene mensajes genéricos para no revelar si un usuario existe.

Configuraciones tiene las secciones a la izquierda, agrupadas en Biblioteca, Acceso, Editor, Integraciones y Datos, con el buscador **Buscar ajuste** para filtrarlas. A la derecha, cada sección muestra su descripción y sus ajustes en tarjetas, con interruptores, deslizadores con su valor y el estado de cada prueba de conexión. En ventanas angostas, las secciones pasan a una tira desplazable arriba del contenido. Configuraciones mide como máximo 1280 × 820 píxeles y se achica en pantallas más chicas. Administrar librerías y el selector de archivos del chat ocupan el 75 % de la ventana (casi toda la pantalla en teléfonos). Los demás modales, como los mensajes de error y las confirmaciones, se ajustan a su contenido.

Los desplegables de Configuraciones tienen el mismo comportamiento de menú que el resto de Notia: se cierran al seleccionar, con Escape o al hacer click fuera, y se pueden recorrer con teclado.

Si escribis al bot desde un chat sin usuario vinculado, Notia te indica que inicies sesion con `/start` para comenzar el enlace y no procesa el mensaje como una consulta.

El primer enlace de Telegram también funciona para usuarios recién creados sin contraseña: después de confirmar la nueva contraseña, Notia guarda el acceso y vincula el chat.
