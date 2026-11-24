; Coloration CSS de JulieStorm. Écrite pour nos catégories : les sélecteurs ont
; leur propre couleur, distincte des propriétés.

(comment) @comment

; Sélecteurs : tout le sélecteur, puis ce qui s'y distingue.
(tag_name) @selector
(nesting_selector) @selector
(universal_selector) @selector
(class_selector) @selector
(id_selector) @selector
(pseudo_class_selector) @selector
(pseudo_element_selector) @selector
(attribute_selector (attribute_name) @attribute)
(attribute_selector (plain_value) @string)

; Déclarations
(property_name) @property
((property_name) @variable
 (#match? @variable "^--"))
((plain_value) @variable
 (#match? @variable "^--"))
(function_name) @function

; Règles @
(at_keyword) @keyword
"@media" @keyword
"@import" @keyword
"@charset" @keyword
"@namespace" @keyword
"@supports" @keyword
"@keyframes" @keyword
(to) @keyword
(from) @keyword
(important) @keyword
"and" @keyword
"or" @keyword
"not" @keyword
"only" @keyword

; Valeurs
(string_value) @string
(color_value) @number
(integer_value) @number
(float_value) @number
(unit) @number
