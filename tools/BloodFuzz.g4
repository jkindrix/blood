// Blood Language Grammar for Fuzzing (ANTLR v4)
// Core subset targeting effects, generics, closures, patterns, regions.
// Not a complete grammar — optimized for test generation coverage.

grammar BloodFuzz;

// ─── Parser Rules ────────────────────────────────────────────────────────

program
    : moduleDecl? item* mainFn EOF
    ;

moduleDecl
    : 'module' IDENT ('.' IDENT)* ';'
    ;

item
    : fnDecl
    | structDecl
    | enumDecl
    | effectDecl
    | handlerDecl
    | traitDecl
    | implBlock
    ;

// ─── Functions ───────────────────────────────────────────────────────────

fnDecl
    : 'pub'? 'fn' IDENT typeParams? '(' params ')' ('->' type_)? effectAnnot? whereClause? block
    ;

mainFn
    : 'fn' 'main' '(' ')' '->' 'i32' block
    ;

typeParams
    : '<' typeParam (',' typeParam)* '>'
    ;

typeParam
    : IDENT (':' typeBound)?
    ;

typeBound
    : type_ ('+' type_)*
    ;

params
    : (param (',' param)*)?
    ;

param
    : paramQualifier? IDENT ':' type_
    | '&' 'self'
    | '&' 'mut' 'self'
    | 'self'
    ;

paramQualifier
    : 'linear'
    | 'affine'
    | 'mut'
    ;

effectAnnot
    : '/' '{' UPPER_IDENT (',' UPPER_IDENT)* '}'
    | '/' 'pure'
    ;

whereClause
    : 'where' wherePred (',' wherePred)*
    ;

wherePred
    : type_ ':' typeBound
    ;

// ─── Types ───────────────────────────────────────────────────────────────

type_
    : primitiveType
    | UPPER_IDENT typeArgs?
    | '&' 'mut'? type_
    | 'fn' '(' typeList? ')' '->' type_ effectAnnot?
    | '[' type_ ']'
    | '[' type_ ';' INT_LITERAL ']'
    | '(' type_ (',' type_)+ ')'
    | '(' ')'
    | 'Vec' '<' type_ '>'
    | 'HashMap' '<' type_ ',' type_ '>'
    | 'Option' '<' type_ '>'
    | 'String'
    | 'linear' type_
    | 'affine' type_
    ;

primitiveType
    : 'i32' | 'i64' | 'u32' | 'u64' | 'i8' | 'u8' | 'i16' | 'u16'
    | 'f32' | 'f64' | 'bool' | 'usize' | 'isize' | 'char'
    ;

typeArgs
    : '<' type_ (',' type_)* '>'
    ;

typeList
    : type_ (',' type_)*
    ;

// ─── Structs & Enums ─────────────────────────────────────────────────────

structDecl
    : 'pub'? 'struct' UPPER_IDENT typeParams? '{' structFields '}'
    ;

structFields
    : (structField (',' structField)* ','?)?
    ;

structField
    : 'pub'? IDENT ':' type_
    ;

enumDecl
    : 'pub'? 'enum' UPPER_IDENT typeParams? '{' enumVariants '}'
    ;

enumVariants
    : enumVariant (',' enumVariant)* ','?
    ;

enumVariant
    : UPPER_IDENT ('(' typeList ')')?
    ;

// ─── Effects & Handlers ──────────────────────────────────────────────────

effectDecl
    : 'effect' UPPER_IDENT typeParams? '{' operationDecl+ '}'
    ;

operationDecl
    : 'op' IDENT '(' params ')' '->' type_ ';'
    ;

handlerDecl
    : handlerKind? 'handler' UPPER_IDENT typeParams? 'for' UPPER_IDENT typeArgs?
      '{' handlerBody '}'
    ;

handlerKind
    : 'deep'
    | 'shallow'
    ;

handlerBody
    : handlerState* returnClause? operationImpl*
    ;

handlerState
    : 'let' 'mut'? IDENT ':' type_ ('=' expr)? ';'?
    ;

returnClause
    : 'return' '(' IDENT ')' block
    ;

operationImpl
    : 'op' IDENT '(' params ')' block
    ;

// ─── Traits & Impls ──────────────────────────────────────────────────────

traitDecl
    : 'pub'? 'trait' UPPER_IDENT typeParams? (':' typeBound)? '{' traitItem* '}'
    ;

traitItem
    : 'fn' IDENT typeParams? '(' params ')' ('->' type_)? (block | ';')
    ;

implBlock
    : 'impl' typeParams? UPPER_IDENT typeArgs? 'for' type_ '{' implItem* '}'
    | 'impl' typeParams? UPPER_IDENT typeArgs? '{' implItem* '}'
    ;

implItem
    : 'pub'? 'fn' IDENT typeParams? '(' params ')' ('->' type_)? block
    ;

// ─── Statements ──────────────────────────────────────────────────────────

block
    : '{' stmt* expr? '}'
    ;

stmt
    : 'let' 'mut'? IDENT (':' type_)? '=' expr ';'
    | expr ';'
    | 'return' expr ';'
    | 'while' expr block
    | forLoop
    ;

forLoop
    : 'for' IDENT 'in' expr block
    ;

// ─── Expressions ─────────────────────────────────────────────────────────

expr
    : primary
    | expr '.' IDENT ('(' args ')')?
    | expr '[' expr ']'
    | expr binOp expr
    | '!' expr
    | '-' expr
    | '&' 'mut'? expr
    | '*' expr
    | expr 'as' type_
    | ifExpr
    | matchExpr
    | closureExpr
    | performExpr
    | handlerExpr
    | regionExpr
    ;

primary
    : INT_LITERAL
    | FLOAT_LITERAL
    | STRING_LITERAL
    | CHAR_LITERAL
    | 'true'
    | 'false'
    | IDENT ('(' args ')')?
    | UPPER_IDENT '.' UPPER_IDENT ('(' args ')')?
    | UPPER_IDENT '{' fieldInits '}'
    | 'vec' '!' '[' args ']'
    | 'format' '!' '(' STRING_LITERAL (',' args)? ')'
    | 'println' '!' '(' STRING_LITERAL (',' args)? ')'
    | '(' expr ')'
    | block
    ;

args
    : (expr (',' expr)*)?
    ;

fieldInits
    : (fieldInit (',' fieldInit)* ','?)?
    ;

fieldInit
    : IDENT ':' expr
    ;

binOp
    : '+' | '-' | '*' | '/' | '%'
    | '==' | '!=' | '<' | '>' | '<=' | '>='
    | '&&' | '||'
    | '|>'
    ;

ifExpr
    : 'if' expr block ('else' (ifExpr | block))?
    ;

matchExpr
    : 'match' expr '{' matchArm (',' matchArm)* ','? '}'
    ;

matchArm
    : pattern '=>' expr
    ;

pattern
    : '_'
    | IDENT
    | INT_LITERAL
    | 'true'
    | 'false'
    | '&' pattern
    | UPPER_IDENT '.' UPPER_IDENT ('(' patternList ')')?
    | UPPER_IDENT '{' fieldPatterns '}'
    | '(' patternList ')'
    ;

patternList
    : pattern (',' pattern)*
    ;

fieldPatterns
    : (fieldPattern (',' fieldPattern)* ','?)?
    ;

fieldPattern
    : IDENT (':' pattern)?
    ;

closureExpr
    : '|' closureParams '|' ('->' type_)? block
    | '|' closureParams '|' ('->' type_)? expr
    | 'move' '|' closureParams '|' ('->' type_)? block
    ;

closureParams
    : (closureParam (',' closureParam)*)?
    ;

closureParam
    : IDENT (':' type_)?
    ;

performExpr
    : 'perform' UPPER_IDENT '.' IDENT '(' args ')'
    ;

handlerExpr
    : 'with' UPPER_IDENT typeArgs? '{' fieldInits '}' 'handle' block
    ;

regionExpr
    : 'region' block
    ;

// ─── Lexer Rules ─────────────────────────────────────────────────────────

UPPER_IDENT : [A-Z] [a-zA-Z0-9_]* ;
IDENT       : [a-z_] [a-zA-Z0-9_]* ;
INT_LITERAL : [0-9] [0-9_]* ;
FLOAT_LITERAL : [0-9] [0-9_]* '.' [0-9] [0-9_]* ;
STRING_LITERAL : '"' (~["\\\r\n] | '\\' .)* '"' ;
CHAR_LITERAL : '\'' (~['\\\r\n] | '\\' .) '\'' ;

LINE_COMMENT : '//' ~[\r\n]* -> skip ;
BLOCK_COMMENT : '/*' .*? '*/' -> skip ;
WS : [ \t\r\n]+ -> skip ;
