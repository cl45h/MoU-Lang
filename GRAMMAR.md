# Mou: gramática (v0.1, borrador)

Notación EBNF: `a?` opcional, `a*` cero o más, `a+` uno o más, `a | b` alternativa, `"x"` terminal.
`lista(X)` abrevia `(X ("," X)* ","?)?`.

## 1. Léxico

```ebnf
ident      = (letra | "_") (letra | digito | "_")* ;
entero     = digito (digito | "_")* | "0x" hex+ | "0b" bin+ | "0o" oct+ ;
flotante   = digito+ "." digito+ (("e" | "E") ("+" | "-")? digito+)? ;
cadena     = '"' (caracter | escape | "{" expr "}")* '"' ;
caracter_l = "'" (caracter | escape) "'" ;
comentario = "//" hasta_fin_de_linea | "/*" ... "*/" (anidables) ;
```

Palabras reservadas:

```
as break catch comptime continue defer dyn else enum false fn for if impl import
in let loop match mut own pub ref region return scope self Self soa struct trait
true type unsafe var where while with
```

### Fin de sentencia

No hay punto y coma. Un salto de línea termina la sentencia salvo que:
- esté dentro de `( )` o `[ ]`, o
- la línea termine en un signo que no sea `)`, `]`, `}` ni `?` (un operador, `,`, `=`, `{`, `->`, `=>`, etc.), o
- la línea siguiente empiece con `.`, `?`, `,`, un operador binario, o con `else`, `catch`, `where` o `as`.

Un `-` al inicio de línea siempre empieza una sentencia nueva (es el único operador binario que también es unario).

En la gramática, `nl` representa ese fin de sentencia. Dentro de `{ }`, las listas (campos, variantes, brazos de `match`) aceptan un salto de línea en lugar de la coma.

## 2. Archivo y declaraciones

```ebnf
archivo     = perfil? import* decl* ;
perfil      = "@profile" "(" ("script" | "app" | "system") ")" nl ;

import      = "import" (ruta | origen cadena) ("as" ident)? nl ;
origen      = "c" | "cpp" | "py" ;
ruta        = ident ("." ident)* ;

decl        = atributo* "pub"? (fn_decl | struct_decl | enum_decl | trait_decl
                               | impl_decl | const_decl | type_decl) ;
atributo    = "@" ident ("(" lista(expr) ")")? ;

fn_decl     = "fn" ident genericos? "(" lista(param) ")" retorno? where? (bloque | "=" expr nl | nl) ;
param       = "comptime"? ident (":" modo? tipo)? ("=" expr)? | modo? "self" ;
modo        = "mut" | "own" ;
retorno     = ("->" "ref"? tipo)? ("!" tipo)? ;
where       = "where" lista(expr) ;

genericos   = "[" lista(gen_param) "]" ;
gen_param   = ident (":" cota)? | "region" ident | "comptime" ident ":" tipo ;
cota        = tipo ("+" tipo)* ;

struct_decl = "struct" ident genericos? "{" lista(campo) "}" ;
campo       = "pub"? ident ":" tipo ("=" expr)? ;

enum_decl   = "enum" ident genericos? "{" lista(variante) "}" ;
variante    = ident ("(" lista(campo_var) ")")? ("=" expr)? ;
campo_var   = (ident ":")? tipo ;

trait_decl  = "trait" ident genericos? (":" cota)? "{" fn_decl* "}" ;
impl_decl   = "impl" genericos? (tipo "for")? tipo "{" decl* "}" ;

const_decl  = "let" ident (":" tipo)? "=" expr nl ;      (* nivel de módulo: constante comptime *)
type_decl   = "type" ident genericos? "=" tipo nl ;
```

`impl Trait for Tipo { }` implementa un trait; `impl Tipo { }` agrega métodos.

## 3. Tipos

```ebnf
tipo        = tipo_base ("?")* ;
tipo_base   = ruta ("[" lista(arg_gen) "]")?
            | "[" tipo (";" expr)? "]"                 (* slice o array *)
            | "soa" "[" tipo (";" expr)? "]"
            | "(" lista(tipo) ")"                      (* tupla; () es unidad *)
            | "fn" "(" lista(modo? tipo) ")" retorno
            | "ref" ("[" ident "]")? "mut"? tipo       (* solo retornos y perfil system *)
            | "dyn" cota
            | "Self" | "type" ;
arg_gen     = tipo | expr ;
```

`T ! E` solo aparece en posición de retorno (`retorno`); como tipo de valor se escribe `Result[T, E]`.

## 4. Sentencias

```ebnf
bloque      = "{" sentencia* expr? "}" ;
sentencia   = let_stmt | asignacion | defer_stmt | expr nl | decl ;

let_stmt    = ("let" | "var") patron (":" tipo)? "=" expr nl ;
asignacion  = expr op_asig expr nl ;
op_asig     = "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" ;
defer_stmt  = "defer" (expr nl | bloque) ;
```

## 5. Expresiones

Precedencia de menor a mayor:

| Nivel | Operadores | Asociatividad |
|---|---|---|
| 1 | `catch` | izquierda |
| 2 | `\|\|` | izquierda |
| 3 | `&&` | izquierda |
| 4 | `== != < <= > >=` | no asociativos |
| 5 | `..` `..=` | no asociativos |
| 6 | `\|` | izquierda |
| 7 | `^` | izquierda |
| 8 | `&` | izquierda |
| 9 | `<< >>` | izquierda |
| 10 | `+ - +% -% +\| -\|` | izquierda |
| 11 | `* / % *% *\|` | izquierda |
| 12 | `as` | izquierda |
| 13 | unarios `- ! ~` | prefijo |
| 14 | postfijos `() [] . ?` | izquierda |

```ebnf
expr        = expr_bin ("catch" ident? bloque)? ;
expr_bin    = unaria (op_bin unaria)* ;                (* según la tabla *)
unaria      = ("-" | "!" | "~") unaria | postfija ;
postfija    = primaria sufijo* ;
sufijo      = "(" lista(argumento) ")"
            | "[" lista(expr) "]"
            | "." (ident | entero)
            | "?" ;
argumento   = (ident ":")? modo? expr ;

primaria    = literal | ruta | "self"
            | "(" lista(expr) ")"                      (* agrupación o tupla *)
            | "[" lista(expr) "]" | "[" expr ";" expr "]"
            | bloque
            | if_expr | match_expr | bucle
            | "fn" "(" lista(param) ")" retorno bloque (* closure *)
            | "comptime" (bloque | unaria)
            | "unsafe" bloque
            | "scope" ident bloque
            | "with" lista(ident "=" expr) bloque
            | "return" expr? | "break" expr? | "continue" ;

if_expr     = "if" condicion bloque ("else" (if_expr | bloque))? ;
condicion   = expr | "let" patron "=" expr ;
match_expr  = "match" expr "{" lista(brazo) "}" ;
brazo       = patron ("if" expr)? "=>" expr ;
bucle       = "for" patron "in" expr bloque
            | "while" condicion bloque
            | "loop" bloque ;

literal     = entero | flotante | cadena | caracter_l | "true" | "false" ;
```

## 6. Patrones

```ebnf
patron      = patron_alt ("|" patron_alt)* ;
patron_alt  = "_"
            | literal (("..") | ("..=")) literal?
            | "mut"? ident                             (* enlace *)
            | ruta "(" lista(patron) ")"               (* variante o struct posicional *)
            | ruta "{" lista(ident (":" patron)?) ".."? "}"
            | "(" lista(patron) ")"
            | "[" lista(patron | "..") "]" ;
```

## 7. Resoluciones de ambigüedad

1. **`expr[...]`**: el parser no distingue entre indexado (`xs[i]`) y aplicación genérica (`max[int]`). Produce un nodo único `Indice` que la resolución de nombres clasifica: si la base es un tipo o una función genérica, es aplicación genérica.
2. **Literales de struct** usan sintaxis de llamada: `Point(1, 2)` o `Point(x: 1, y: 2)`. No existe `Point { ... }` como expresión, así que `if x { ... }` nunca es ambiguo.
3. **`!` en retorno** contra `!` unario: en `retorno` solo aparece después de `)` o de un tipo, donde un unario no es posible.
4. **`?` de tipo** contra `?` de propagación: uno solo aparece en posición de tipo y el otro en posición de expresión; el parser siempre sabe en cuál está.
5. **Bloque contra expresión al inicio de sentencia**: una sentencia que empieza con `{` es siempre un bloque.
6. **`|` en patrones** contra OR binario: dentro de `patron` siempre es alternativa.
7. **Enlace contra constructor en patrones**: un nombre suelto en mayúscula inicial (`None`, `Less`) o con punto (`Order.Less`) es un constructor; en minúscula es un enlace de variable.
8. **`ref` en el retorno**: `-> ref T` es retorno por referencia; `-> ref[r] T` y `-> ref mut T` son tipos referencia. En un tipo, `ref[r] T` nombra una región solo si después de `]` empieza otro tipo; si no, `ref [T]` es una referencia a un slice.
9. **Atributos de layout** van como prefijo: `@c struct`, `@packed struct`.

## 8. Fuera de esta versión del parser

- Tipos anónimos como expresión (`struct { ... }` devuelto desde una función `comptime`).
- Bloques `comptime { }` a nivel de módulo.
- Especificadores de formato dentro de la interpolación de strings.
