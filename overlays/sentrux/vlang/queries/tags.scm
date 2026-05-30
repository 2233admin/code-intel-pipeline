; V language tags.scm

(function_declaration
  name: (identifier) @name) @definition.function

(function_declaration
  name: (binded_identifier) @name) @definition.function

(struct_declaration
  name: (type_identifier) @name) @definition.class

(enum_declaration
  name: (type_identifier) @name) @definition.class

(interface_declaration
  name: (type_identifier) @name) @definition.interface

; Calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (selector_expression
    field: (identifier) @name)) @reference.call

(type_identifier) @reference.type

; Imports
(import_declaration
  path: (import_path) @import.module) @import
