(** * Blood Core Calculus — Syntax

    This file defines the abstract syntax of Blood's core calculus,
    including expressions, types, values, and effect rows.

    Reference: FORMAL_SEMANTICS.md §1 (Syntax)
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
Import ListNotations.

(** ** Variables

    We use de Bruijn indices for bound variables to avoid capture issues
    in substitution. *)

Definition var := nat.

(** ** Labels (for records and effect operations) *)

Definition label := string.

(** ** Effect names *)

Definition effect_name := string.

(** ** Operation names *)

Definition op_name := string.

(** ** Base types

    Corresponds to Blood's primitive types. *)

Inductive base_type : Type :=
  | TyI32 : base_type
  | TyI64 : base_type
  | TyI128 : base_type
  | TyBool : base_type
  | TyUnit : base_type
  | TyString : base_type.

(** ** Types

    Corresponds to FORMAL_SEMANTICS.md §1.3.

    T ::= B | T → T / ε | {l₁:T₁,...,lₙ:Tₙ | ρ} | ∀α.T | linear T | affine T | !T *)

Inductive ty : Type :=
  | Ty_Base : base_type -> ty
  | Ty_Arrow : ty -> ty -> effect_row -> ty    (** T → T / ε *)
  | Ty_Record : list (label * ty) -> ty        (** {l₁:T₁, ..., lₙ:Tₙ} (closed record) *)
  | Ty_Forall : ty -> ty                       (** ∀α. T (type var is de Bruijn index 0) *)
  | Ty_Linear : ty -> ty                       (** linear T *)
  | Ty_Affine : ty -> ty                       (** affine T *)
  | Ty_GenRef : ty -> ty                       (** !T (generational reference) *)
  | Ty_Var : nat -> ty                         (** Type variable (de Bruijn index) *)

(** ** Effect rows

    ε ::= {E₁, ..., Eₙ | ρ} | pure

    We represent effect rows as a list of effect entries plus an optional
    row variable (open vs closed rows). *)

with effect_row : Type :=
  | Eff_Pure : effect_row                                (** pure = {} *)
  | Eff_Closed : list effect_entry -> effect_row         (** {E₁, ..., Eₙ} *)
  | Eff_Open : list effect_entry -> nat -> effect_row    (** {E₁, ..., Eₙ | ρ} *)

(** ** Effect entries *)

with effect_entry : Type :=
  | Eff_Entry : effect_name -> effect_entry.

(** ** Constants *)

Inductive constant : Type :=
  | Const_I32 : nat -> constant      (** i32 literal (using nat for simplicity) *)
  | Const_I64 : nat -> constant      (** i64 literal *)
  | Const_Bool : bool -> constant    (** boolean literal *)
  | Const_Unit : constant            (** unit value *)
  | Const_String : string -> constant. (** string literal *)

(** ** Expressions

    Corresponds to FORMAL_SEMANTICS.md §1.1.

    e ::= x | c | λx:T. e | e e | let x = e in e
        | (e : T) | {l₁=e₁,...} | e.l | {l=e|e}
        | perform E.op(e) | with h handle e | resume(e)
*)

Inductive expr : Type :=
  | E_Var : var -> expr                                     (** x *)
  | E_Const : constant -> expr                              (** c *)
  | E_Lam : ty -> expr -> expr                              (** λx:T. e (de Bruijn) *)
  | E_App : expr -> expr -> expr                            (** e₁ e₂ *)
  | E_Let : expr -> expr -> expr                            (** let x = e₁ in e₂ *)
  | E_Annot : expr -> ty -> expr                            (** (e : T) *)
  | E_Record : list (label * expr) -> expr                  (** {l₁=e₁, ..., lₙ=eₙ} *)
  | E_Select : expr -> label -> expr                        (** e.l *)
  | E_Extend : label -> expr -> expr -> expr                (** {l=e₁ | e₂} *)
  | E_Perform : effect_name -> op_name -> expr -> expr      (** perform E.op(e) *)
  | E_Handle : handler -> expr -> expr                      (** with h handle e *)
  | E_Resume : expr -> expr                                 (** resume(e) *)

(** ** Handlers

    A handler for effect E consists of:
    - A return clause: return(x) { e_ret }
    - Operation clauses: op(x) { e_op }
*)

with handler : Type :=
  | Handler : handler_kind -> expr -> list op_clause -> handler

with handler_kind : Type :=
  | Deep : handler_kind
  | Shallow : handler_kind

with op_clause : Type :=
  | OpClause : effect_name -> op_name -> expr -> op_clause.
  (** op_clause E op e_body — the body has resume bound as var 0, arg as var 1 *)

(** ** Values

    Corresponds to FORMAL_SEMANTICS.md §1.2.

    v ::= c | λx:T. e | {l₁=v₁,...,lₙ=vₙ} | (κ, Γ_gen, L)
*)

Inductive value : Type :=
  | V_Const : constant -> value
  | V_Lam : ty -> expr -> value                        (** closure *)
  | V_Record : list (label * value) -> value           (** record value *)
  | V_Continuation : ty -> expr -> gen_snapshot -> value (** continuation = λ(x:T).body + snapshot *)

(** ** Generation snapshots

    Corresponds to FORMAL_SEMANTICS.md §4.
*)

with gen_snapshot : Type :=
  | GenSnapshot : list gen_ref -> gen_snapshot

with gen_ref : Type :=
  | GenRef : nat -> nat -> gen_ref.   (** (address, generation) *)

(** ** Effect declarations

    An effect declaration specifies the operations of an effect.
*)

Record effect_decl := mk_effect_decl {
  eff_decl_name : effect_name;
  eff_decl_ops : list (op_name * ty * ty);   (** op : A → B *)
}.

(** ** Decidable equality for labels *)

Definition label_eqb (l1 l2 : label) : bool := String.eqb l1 l2.

Lemma label_eqb_refl : forall l, label_eqb l l = true.
Proof.
  intro l. unfold label_eqb. apply String.eqb_refl.
Qed.

Lemma label_eqb_eq : forall l1 l2, label_eqb l1 l2 = true <-> l1 = l2.
Proof.
  intros l1 l2. unfold label_eqb. apply String.eqb_eq.
Qed.

(** ** Decidable equality for effect names *)

Definition effect_name_eqb := String.eqb.

(** ** Decidable equality for operation names *)

Definition op_name_eqb := String.eqb.

(** ** Type of a constant *)

Definition typeof_const (c : constant) : ty :=
  match c with
  | Const_I32 _ => Ty_Base TyI32
  | Const_I64 _ => Ty_Base TyI64
  | Const_Bool _ => Ty_Base TyBool
  | Const_Unit => Ty_Base TyUnit
  | Const_String _ => Ty_Base TyString
  end.

(** ** Value embedding into expressions *)

Fixpoint value_to_expr (v : value) : expr :=
  match v with
  | V_Const c => E_Const c
  | V_Lam t e => E_Lam t e
  | V_Record fields =>
      E_Record (map (fun '(l, v) => (l, value_to_expr v)) fields)
  | V_Continuation t body _ => E_Lam t body   (** continuation is a lambda with snapshot *)
  end.

(** ** Predicate: is an expression a value? *)

Fixpoint is_value (e : expr) : bool :=
  match e with
  | E_Const _ => true
  | E_Lam _ _ => true
  | E_Record fields => forallb (fun '(_, e) => is_value e) fields
  | _ => false
  end.

(** ** Effect row membership *)

Definition effect_in_row (eff : effect_name) (row : effect_row) : Prop :=
  match row with
  | Eff_Pure => False
  | Eff_Closed entries =>
      In (Eff_Entry eff) entries
  | Eff_Open entries _ =>
      In (Eff_Entry eff) entries  (** conservative: only check concrete entries *)
  end.

(** ** Effect row subset *)

Definition effect_row_subset (r1 r2 : effect_row) : Prop :=
  match r1 with
  | Eff_Pure => True  (** pure is subset of everything *)
  | Eff_Closed entries1 =>
      match r2 with
      | Eff_Pure => entries1 = []
      | Eff_Closed entries2 =>
          forall e, In e entries1 -> In e entries2
      | Eff_Open entries2 _ =>
          forall e, In e entries1 -> In e entries2
      end
  | Eff_Open entries1 _ =>
      match r2 with
      | Eff_Pure => False  (** open row can't be subset of pure *)
      | Eff_Closed _ => False  (** open can't be subset of closed *)
      | Eff_Open entries2 _ =>
          (** Entry-level inclusion only. Row variables are uninterpreted
              in this formalization (no unification), so we compare only
              the concrete effect entries. This is sound: if all concrete
              effects in r1 are in r2, then r1's known effects are covered. *)
          forall e, In e entries1 -> In e entries2
      end
  end.

(** ** Effect row union *)

Fixpoint effect_entries_union (es1 es2 : list effect_entry) : list effect_entry :=
  match es1 with
  | [] => es2
  | (Eff_Entry n) :: rest =>
      if existsb (fun e => match e with Eff_Entry n' => effect_name_eqb n n' end) es2
      then effect_entries_union rest es2
      else (Eff_Entry n) :: effect_entries_union rest es2
  end.

Definition effect_row_union (r1 r2 : effect_row) : effect_row :=
  match r1, r2 with
  | Eff_Pure, r => r
  | r, Eff_Pure => r
  | Eff_Closed es1, Eff_Closed es2 =>
      Eff_Closed (effect_entries_union es1 es2)
  | Eff_Closed es1, Eff_Open es2 rv =>
      Eff_Open (effect_entries_union es1 es2) rv
  | Eff_Open es1 rv, Eff_Closed es2 =>
      Eff_Open (effect_entries_union es1 es2) rv
  | Eff_Open es1 rv1, Eff_Open es2 rv2 =>
      (** Simplification: take first row variable *)
      Eff_Open (effect_entries_union es1 es2) rv1
  end.

(** ** Effect entry union algebraic properties *)

Lemma effect_entries_union_in_right :
  forall es1 es2 e,
    In e es2 -> In e (effect_entries_union es1 es2).
Proof.
  induction es1 as [| [n] rest IH]; intros es2 e Hin.
  - exact Hin.
  - simpl.
    destruct (existsb (fun e0 => match e0 with Eff_Entry n' => effect_name_eqb n n' end) es2).
    + exact (IH es2 e Hin).
    + right. exact (IH es2 e Hin).
Qed.

Lemma effect_entries_union_in_left :
  forall es1 es2 e,
    In e es1 -> In e (effect_entries_union es1 es2).
Proof.
  induction es1 as [| [n] rest IH]; intros es2 e Hin.
  - inversion Hin.
  - simpl in Hin. destruct Hin as [Heq | Hin].
    + subst e. simpl.
      destruct (existsb (fun e0 => match e0 with Eff_Entry n' => effect_name_eqb n n' end) es2) eqn:Hex.
      * apply existsb_exists in Hex.
        destruct Hex as [[n'] [Hin' Heqb]].
        unfold effect_name_eqb in Heqb.
        apply String.eqb_eq in Heqb. subst n'.
        apply effect_entries_union_in_right. exact Hin'.
      * simpl. left. reflexivity.
    + simpl.
      destruct (existsb _ es2).
      * exact (IH es2 e Hin).
      * right. exact (IH es2 e Hin).
Qed.

Lemma effect_entries_union_in_or :
  forall es1 es2 e,
    In e (effect_entries_union es1 es2) ->
    In e es1 \/ In e es2.
Proof.
  induction es1 as [| [n] rest IH]; intros es2 e Hin.
  - right. exact Hin.
  - simpl in Hin.
    destruct (existsb (fun e0 => match e0 with Eff_Entry n' => effect_name_eqb n n' end) es2) eqn:Hex.
    + destruct (IH es2 e Hin) as [Hr | Hl].
      * left. right. exact Hr.
      * right. exact Hl.
    + simpl in Hin. destruct Hin as [Heq | Hin].
      * left. left. exact Heq.
      * destruct (IH es2 e Hin) as [Hr | Hl].
        -- left. right. exact Hr.
        -- right. exact Hl.
Qed.

Lemma effect_entries_union_intro :
  forall es1 es2 e,
    In e es1 \/ In e es2 ->
    In e (effect_entries_union es1 es2).
Proof.
  intros es1 es2 e [Hin | Hin].
  - exact (effect_entries_union_in_left es1 es2 e Hin).
  - exact (effect_entries_union_in_right es1 es2 e Hin).
Qed.

(** Membership in effect_row_union from left component *)

Lemma effect_in_union_left :
  forall eff_nm eff1 eff2,
    effect_in_row eff_nm eff1 ->
    effect_in_row eff_nm (effect_row_union eff1 eff2).
Proof.
  intros eff_nm eff1 eff2 Hin.
  destruct eff1; simpl in *.
  - (* Pure *) contradiction.
  - (* Closed *)
    destruct eff2; simpl.
    + exact Hin.
    + apply effect_entries_union_in_left. exact Hin.
    + apply effect_entries_union_in_left. exact Hin.
  - (* Open *)
    destruct eff2; simpl.
    + exact Hin.
    + apply effect_entries_union_in_left. exact Hin.
    + apply effect_entries_union_in_left. exact Hin.
Qed.

Lemma effect_in_union_right :
  forall eff_nm eff1 eff2,
    effect_in_row eff_nm eff2 ->
    effect_in_row eff_nm (effect_row_union eff1 eff2).
Proof.
  intros eff_nm eff1 eff2 Hin.
  destruct eff2; simpl in *.
  - (* Pure *) contradiction.
  - (* Closed *)
    destruct eff1; simpl.
    + exact Hin.
    + apply effect_entries_union_in_right. exact Hin.
    + apply effect_entries_union_in_right. exact Hin.
  - (* Open *)
    destruct eff1; simpl.
    + exact Hin.
    + apply effect_entries_union_in_right. exact Hin.
    + apply effect_entries_union_in_right. exact Hin.
Qed.

(** Subset preserves membership *)

Lemma effect_in_row_subset :
  forall eff_nm eff eff',
    effect_in_row eff_nm eff ->
    effect_row_subset eff eff' ->
    effect_in_row eff_nm eff'.
Proof.
  intros eff_nm eff eff' Hin Hsub.
  destruct eff, eff'; simpl in *;
    try contradiction;
    try (subst; inversion Hin; fail).
  - (* Closed/Closed *) apply Hsub. exact Hin.
  - (* Closed/Open *) apply Hsub. exact Hin.
  - (* Open/Open *) apply Hsub. exact Hin.
Qed.

(** ** Custom nested induction principle for expr/handler/op_clause

    Standard [induction e] does not provide induction hypotheses for
    expressions nested inside lists (E_Record fields, Handler clauses).
    This principle provides [Forall]-based IH for list elements. *)

Section expr_nested_ind.
  Variable P : expr -> Prop.
  Variable P_handler : handler -> Prop.
  Variable P_clause : op_clause -> Prop.

  Hypothesis H_Var : forall v, P (E_Var v).
  Hypothesis H_Const : forall c, P (E_Const c).
  Hypothesis H_Lam : forall T body, P body -> P (E_Lam T body).
  Hypothesis H_App : forall e1 e2, P e1 -> P e2 -> P (E_App e1 e2).
  Hypothesis H_Let : forall e1 e2, P e1 -> P e2 -> P (E_Let e1 e2).
  Hypothesis H_Annot : forall e T, P e -> P (E_Annot e T).
  Hypothesis H_Record : forall fields,
    Forall (fun p => P (snd p)) fields -> P (E_Record fields).
  Hypothesis H_Select : forall e l, P e -> P (E_Select e l).
  Hypothesis H_Extend : forall l e1 e2,
    P e1 -> P e2 -> P (E_Extend l e1 e2).
  Hypothesis H_Perform : forall eff op e, P e -> P (E_Perform eff op e).
  Hypothesis H_Handle : forall h e,
    P_handler h -> P e -> P (E_Handle h e).
  Hypothesis H_Resume : forall e, P e -> P (E_Resume e).
  Hypothesis H_Handler : forall hk e_ret clauses,
    P e_ret -> Forall P_clause clauses ->
    P_handler (Handler hk e_ret clauses).
  Hypothesis H_OpClause : forall eff op body,
    P body -> P_clause (OpClause eff op body).

  Fixpoint expr_nested_ind (e : expr) : P e :=
    match e return P e with
    | E_Var v => H_Var v
    | E_Const c => H_Const c
    | E_Lam T body => H_Lam T body (expr_nested_ind body)
    | E_App e1 e2 =>
        H_App e1 e2 (expr_nested_ind e1) (expr_nested_ind e2)
    | E_Let e1 e2 =>
        H_Let e1 e2 (expr_nested_ind e1) (expr_nested_ind e2)
    | E_Annot e1 T => H_Annot e1 T (expr_nested_ind e1)
    | E_Record fields =>
        H_Record fields
          ((fix fields_ind (fs : list (label * expr))
              : Forall (fun p => P (snd p)) fs :=
            match fs return Forall (fun p => P (snd p)) fs with
            | [] => Forall_nil _
            | (l, ei) :: rest =>
                Forall_cons (l, ei) (expr_nested_ind ei) (fields_ind rest)
            end) fields)
    | E_Select e1 l => H_Select e1 l (expr_nested_ind e1)
    | E_Extend l e1 e2 =>
        H_Extend l e1 e2 (expr_nested_ind e1) (expr_nested_ind e2)
    | E_Perform eff op e1 =>
        H_Perform eff op e1 (expr_nested_ind e1)
    | E_Handle h e1 =>
        H_Handle h e1 (handler_nested_ind h) (expr_nested_ind e1)
    | E_Resume e1 => H_Resume e1 (expr_nested_ind e1)
    end
  with handler_nested_ind (h : handler) : P_handler h :=
    match h return P_handler h with
    | Handler hk e_ret clauses =>
        H_Handler hk e_ret clauses (expr_nested_ind e_ret)
          ((fix clauses_ind (cs : list op_clause)
              : Forall P_clause cs :=
            match cs return Forall P_clause cs with
            | [] => Forall_nil _
            | c :: rest =>
                Forall_cons c (op_clause_nested_ind c) (clauses_ind rest)
            end) clauses)
    end
  with op_clause_nested_ind (cl : op_clause) : P_clause cl :=
    match cl return P_clause cl with
    | OpClause eff op body =>
        H_OpClause eff op body (expr_nested_ind body)
    end.

End expr_nested_ind.

(** ** Notation *)

Notation "'pure'" := Eff_Pure (at level 0).
