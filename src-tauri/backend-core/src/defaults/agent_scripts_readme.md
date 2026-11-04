# Scripts de soporte

Este archivo lo mantiene Notia: se lee pero no se edita, y vuelve a aparecer si se borra.

En esta carpeta viven los scripts Python que el asistente usa como herramientas. Algunos los escribe él cuando un trabajo es difícil o repetitivo y no tiene una herramienta para hacerlo; otros los podés agregar vos.

## Cómo agregar uno

1. Creá un archivo `nombre.py` en esta carpeta. El nombre va en minúsculas, con letras, números y `_`, y empieza con una letra (por ejemplo `csv_a_tabla.py`): es el nombre con el que lo ve el asistente.
2. Empezalo con un docstring que diga qué hace y qué parámetros recibe:

```python
"""
---
description: Convierte un CSV de la biblioteca en una tabla Markdown.
parameters:
  - path (string, requerido): ruta del CSV dentro de la biblioteca
  - delimiter (string): separador; por defecto ","
---
"""
import csv, json, os, sys

args = json.load(sys.stdin)
ruta = os.path.join(os.environ["NOTIA_LIBRARY"], args["path"])
with open(ruta, newline="", encoding="utf-8") as archivo:
    filas = list(csv.reader(archivo, delimiter=args.get("delimiter", ",")))
tabla = ["| " + " | ".join(fila) + " |" for fila in filas]
if len(tabla) > 1:
    tabla.insert(1, "|" + " --- |" * len(filas[0]))
print(json.dumps({"tabla": "\n".join(tabla)}, ensure_ascii=False))
```

Los tipos de parámetro son `string`, `number`, `integer`, `boolean`, `array` y `object`; `requerido` marca los obligatorios.

## Cómo corre

- Recibe los argumentos como un objeto JSON por la entrada estándar (`json.load(sys.stdin)`).
- Corre en la carpeta de la biblioteca, que también está en la variable `NOTIA_LIBRARY`.
- Lo que imprime es lo que lee el asistente: mejor en JSON. Si termina con un código distinto de 0, el asistente lee el error.
- Usa el Python 3 de la PC host; los paquetes que no son de la biblioteca estándar tienen que estar instalados.
- Corre solo en la PC host y solo para el Owner.

El asistente pide confirmación antes de crear o cambiar un script, y te muestra el archivo entero. Correr un script que ya está en esta carpeta no pide confirmación: guardá acá solo código en el que confiás.
