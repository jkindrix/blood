(** * Blood Core Calculus — Typing

    This file defines the typing judgment Γ; Δ ⊢ e : T / ε for Blood's
    core calculus.

    Reference: FORMAL_SEMANTICS.md §5 (Typing Rules)
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
Import ListNotations.
From Blood Require Import Syntax.

(** ** Type context Γ

    Maps variables (de Bruijn indices) to types. *)

Definition type_context := list ty.

(** ** Linearity tracking

    Δ tracks usage multiplicity for linear/affine bindings. *)

Inductive linearity : Type :=
  | Lin_Unrestricted : linearity    (** ordinary: used any number of times *)
  | Lin_Linear : linearity          (** linear: must be used exactly once *)
  | Lin_Affine : linearity.         (** affine: used at most once *)

Definition lin_context := list (linearity * bool).
(** (linearity mode, has_been_used) *)

(** ** Effect signature context Σ

    Maps effect names to their operation signatures. *)

Definition effect_sig := list (op_name * ty * ty).  (** op : A → B *)

Definition effect_context := list (effect_name * effect_sig).

(** ** Context lookup *)

Fixpoint lookup_var (Gamma : type_context) (x : var) : option ty :=
  match Gamma, x with
  | T :: _, 0 => Some T
  | _ :: rest, S n => lookup_var rest n
  | _, _ => None
  end.

Fixpoint lookup_effect (Sigma : effect_context) (eff : effect_name) :
    option effect_sig :=
  match Sigma with
  | [] => None
  | (name, sig) :: rest =>
      if effect_name_eqb name eff then Some sig
      else lookup_effect rest eff
  end.

Fixpoint lookup_op (sig : effect_sig) (op : op_name) : option (ty * ty) :=
  match sig with
  | [] => None
  | (name, arg_ty, ret_ty) :: rest =>
      if op_name_eqb name op then Some (arg_ty, ret_ty)
      else lookup_op rest op
  end.

(** ** Record field lookup *)

Fixpoint lookup_field (fields : list (label * ty)) (l : label) : option ty :=
  match fields with
  | [] => None
  | (l', t) :: rest =>
      if label_eqb l' l then Some t
      else lookup_field rest l
  end.

(** ** Well-formed linearity context *)

Definition lin_context_valid (Delta : lin_context) : Prop :=
  forall i lm used,
    nth_error Delta i = Some (lm, used) ->
    match lm with
    | Lin_Linear => True   (** linear vars will be checked at end of scope *)
    | Lin_Affine => True   (** affine vars may or may not be used *)
    | Lin_Unrestricted => True
    end.

(** ** Linearity context splitting

    Δ = Δ₁ ⊗ Δ₂: each linear binding goes to exactly one side. *)

Inductive lin_split :
    lin_context -> lin_context -> lin_context -> Prop :=
  | Split_Nil :
      lin_split [] [] []
  | Split_Unrestricted : forall Delta Delta1 Delta2 used,
      lin_split Delta Delta1 Delta2 ->
      lin_split ((Lin_Unrestricted, used) :: Delta)
                ((Lin_Unrestricted, used) :: Delta1)
                ((Lin_Unrestricted, used) :: Delta2)
  | Split_Linear_Left : forall Delta Delta1 Delta2 used,
      lin_split Delta Delta1 Delta2 ->
      lin_split ((Lin_Linear, used) :: Delta)
                ((Lin_Linear, used) :: Delta1)
                ((Lin_Linear, true) :: Delta2)
      (** linear binding goes to left; marked used in right *)
  | Split_Linear_Right : forall Delta Delta1 Delta2 used,
      lin_split Delta Delta1 Delta2 ->
      lin_split ((Lin_Linear, used) :: Delta)
                ((Lin_Linear, true) :: Delta1)
                ((Lin_Linear, used) :: Delta2)
      (** linear binding goes to right; marked used in left *)
  | Split_Affine_Left : forall Delta Delta1 Delta2 used,
      lin_split Delta Delta1 Delta2 ->
      lin_split ((Lin_Affine, used) :: Delta)
                ((Lin_Affine, used) :: Delta1)
                ((Lin_Affine, true) :: Delta2)
  | Split_Affine_Right : forall Delta Delta1 Delta2 used,
      lin_split Delta Delta1 Delta2 ->
      lin_split ((Lin_Affine, used) :: Delta)
                ((Lin_Affine, true) :: Delta1)
                ((Lin_Affine, used) :: Delta2)
  | Split_Affine_Neither : forall Delta Delta1 Delta2 used,
      lin_split Delta Delta1 Delta2 ->
      lin_split ((Lin_Affine, used) :: Delta)
                ((Lin_Affine, true) :: Delta1)
                ((Lin_Affine, true) :: Delta2).
      (** affine: neither side uses it *)

(** ** Handler typing

    Handler(E, T, U, ε') where:
    - E = effect being handled
    - T = type of handled computation result
    - U = type of handler result
    - ε' = effects introduced by handler *)

Inductive handler_well_formed :
    effect_context -> type_context -> lin_context ->
    handler -> effect_name -> ty -> ty -> effect_row -> Prop :=
  | HWF : forall Sigma Gamma Delta hk e_ret clauses
           eff_name comp_ty result_ty handler_effects
           eff_sig,
      (** Effect signature lookup *)
      lookup_effect Sigma eff_name = Some eff_sig ->
      (** Return clause types correctly:
          Γ, x:T ⊢ e_ret : U / ε' *)
      has_type Sigma (comp_ty :: Gamma) ((Lin_Unrestricted, false) :: Delta)
               e_ret result_ty handler_effects ->
      (** Each operation clause types correctly *)
      op_clauses_well_formed Sigma Gamma Delta clauses
                             eff_sig result_ty handler_effects ->
      handler_well_formed Sigma Gamma Delta
                          (Handler hk e_ret clauses)
                          eff_name comp_ty result_ty handler_effects

(** ** Operation clause typing *)

with op_clauses_well_formed :
    effect_context -> type_context -> lin_context ->
    list op_clause -> effect_sig -> ty -> effect_row -> Prop :=
  | OpClauses_Nil : forall Sigma Gamma Delta sig result_ty eff,
      op_clauses_well_formed Sigma Gamma Delta [] sig result_ty eff
  | OpClauses_Cons :
      forall Sigma Gamma Delta eff_nm op_nm e_body rest
             sig result_ty handler_eff arg_ty ret_ty,
      lookup_op sig op_nm = Some (arg_ty, ret_ty) ->
      (** Γ, resume:(B→U/ε'), x:A ⊢ e_body : U / ε' *)
      has_type Sigma
               (arg_ty :: Ty_Arrow ret_ty result_ty handler_eff :: Gamma)
               ((Lin_Unrestricted, false) :: (Lin_Unrestricted, false) :: Delta)
               e_body result_ty handler_eff ->
      op_clauses_well_formed Sigma Gamma Delta rest sig result_ty handler_eff ->
      op_clauses_well_formed Sigma Gamma Delta
                             (OpClause eff_nm op_nm e_body :: rest)
                             sig result_ty handler_eff

(** ** Main typing judgment

    Γ; Δ ⊢ e : T / ε

    Reference: FORMAL_SEMANTICS.md §5.2 *)

with has_type :
    effect_context -> type_context -> lin_context ->
    expr -> ty -> effect_row -> Prop :=

  (** [T-Var]
      x : T ∈ Γ
      ──────────────
      Γ; Δ ⊢ x : T / pure *)
  | T_Var : forall Sigma Gamma Delta x T,
      lookup_var Gamma x = Some T ->
      has_type Sigma Gamma Delta (E_Var x) T Eff_Pure

  (** [T-Const]
      ──────────────
      Γ; Δ ⊢ c : typeof(c) / pure *)
  | T_Const : forall Sigma Gamma Delta c,
      has_type Sigma Gamma Delta (E_Const c) (typeof_const c) Eff_Pure

  (** [T-Lam]
      Γ, x:A; Δ,x:○ ⊢ e : B / ε
      ─────────────────────────────
      Γ; Δ ⊢ λx:A. e : A → B / ε / pure *)
  | T_Lam : forall Sigma Gamma Delta A B eff body,
      has_type Sigma (A :: Gamma) ((Lin_Unrestricted, false) :: Delta)
               body B eff ->
      has_type Sigma Gamma Delta (E_Lam A body) (Ty_Arrow A B eff) Eff_Pure

  (** [T-App]
      Γ; Δ₁ ⊢ e₁ : A → B / ε / ε₁    Γ; Δ₂ ⊢ e₂ : A / ε₂
      Δ = Δ₁ ⊗ Δ₂
      ────────────────────────────────────────────
      Γ; Δ ⊢ e₁ e₂ : B / ε ∪ ε₁ ∪ ε₂ *)
  | T_App : forall Sigma Gamma Delta Delta1 Delta2
                   e1 e2 A B fn_eff eff1 eff2,
      lin_split Delta Delta1 Delta2 ->
      has_type Sigma Gamma Delta1 e1 (Ty_Arrow A B fn_eff) eff1 ->
      has_type Sigma Gamma Delta2 e2 A eff2 ->
      has_type Sigma Gamma Delta (E_App e1 e2) B
               (effect_row_union fn_eff (effect_row_union eff1 eff2))

  (** [T-Let]
      Γ; Δ₁ ⊢ e₁ : A / ε₁    Γ, x:A; Δ₂, x:○ ⊢ e₂ : B / ε₂
      Δ = Δ₁ ⊗ Δ₂
      ──────────────────────────────────────────────
      Γ; Δ ⊢ let x = e₁ in e₂ : B / ε₁ ∪ ε₂ *)
  | T_Let : forall Sigma Gamma Delta Delta1 Delta2
                   e1 e2 A B eff1 eff2,
      lin_split Delta Delta1 Delta2 ->
      has_type Sigma Gamma Delta1 e1 A eff1 ->
      has_type Sigma (A :: Gamma) ((Lin_Unrestricted, false) :: Delta2)
               e2 B eff2 ->
      has_type Sigma Gamma Delta (E_Let e1 e2) B
               (effect_row_union eff1 eff2)

  (** [T-Annot]
      Γ; Δ ⊢ e : T / ε
      ────────────────────
      Γ; Δ ⊢ (e : T) : T / ε *)
  | T_Annot : forall Sigma Gamma Delta e T eff,
      has_type Sigma Gamma Delta e T eff ->
      has_type Sigma Gamma Delta (E_Annot e T) T eff

  (** [T-Record]
      Γ; Δᵢ ⊢ eᵢ : Tᵢ / εᵢ
      ─────────────────────────────────
      Γ; Δ ⊢ {l₁=e₁,...} : {l₁:T₁,...} / ε₁∪...∪εₙ

      Simplified: all fields share the same context (no splitting). *)
  | T_Record : forall Sigma Gamma Delta fields field_types eff,
      record_fields_typed Sigma Gamma Delta fields field_types eff ->
      has_type Sigma Gamma Delta (E_Record fields)
               (Ty_Record field_types) eff

  (** [T-Select]
      Γ; Δ ⊢ e : {l₁:T₁,...,lₙ:Tₙ | ρ}    l = lᵢ
      ──────────────────────────────────────────
      Γ; Δ ⊢ e.l : Tᵢ / ε *)
  | T_Select : forall Sigma Gamma Delta e l T fields eff,
      has_type Sigma Gamma Delta e (Ty_Record fields) eff ->
      lookup_field fields l = Some T ->
      has_type Sigma Gamma Delta (E_Select e l) T eff

  (** [T-Perform]
      effect E { op : A → B } ∈ Σ    E ∈ ε
      Γ; Δ ⊢ e : A / ε'
      ────────────────────────────
      Γ; Δ ⊢ perform E.op(e) : B / {E} ∪ ε' *)
  | T_Perform : forall Sigma Gamma Delta e
                       eff_name op eff_sig arg_ty ret_ty eff',
      lookup_effect Sigma eff_name = Some eff_sig ->
      lookup_op eff_sig op = Some (arg_ty, ret_ty) ->
      has_type Sigma Gamma Delta e arg_ty eff' ->
      has_type Sigma Gamma Delta
               (E_Perform eff_name op e) ret_ty
               (effect_row_union
                  (Eff_Closed [Eff_Entry eff_name]) eff')

  (** [T-Handle]
      Γ; Δ₁ ⊢ e : T / {E | ε}
      Γ; Δ₂ ⊢ h : Handler(E, T, U, ε')
      Δ = Δ₁ ⊗ Δ₂
      ────────────────────────────
      Γ; Δ ⊢ with h handle e : U / ε ∪ ε' *)
  | T_Handle : forall Sigma Gamma Delta Delta1 Delta2
                      h e eff_name comp_ty result_ty
                      handler_eff comp_eff,
      lin_split Delta Delta1 Delta2 ->
      has_type Sigma Gamma Delta1 e comp_ty comp_eff ->
      handler_well_formed Sigma Gamma Delta2 h
                          eff_name comp_ty result_ty handler_eff ->
      (** comp_eff should contain eff_name; result removes it *)
      has_type Sigma Gamma Delta (E_Handle h e) result_ty
               (effect_row_union handler_eff comp_eff)
               (** Note: a more precise formulation would subtract
                   eff_name from comp_eff *)

  (** [T-Sub]
      Γ; Δ ⊢ e : T / ε    ε ⊆ ε'
      ──────────────────────────────
      Γ; Δ ⊢ e : T / ε' *)
  | T_Sub : forall Sigma Gamma Delta e T eff eff',
      has_type Sigma Gamma Delta e T eff ->
      effect_row_subset eff eff' ->
      has_type Sigma Gamma Delta e T eff'

(** ** Record field typing (auxiliary) *)

with record_fields_typed :
    effect_context -> type_context -> lin_context ->
    list (label * expr) -> list (label * ty) -> effect_row -> Prop :=
  | RFT_Nil : forall Sigma Gamma Delta,
      record_fields_typed Sigma Gamma Delta [] [] Eff_Pure
  | RFT_Cons : forall Sigma Gamma Delta l e T rest_e rest_t eff1 eff2,
      has_type Sigma Gamma Delta e T eff1 ->
      record_fields_typed Sigma Gamma Delta rest_e rest_t eff2 ->
      record_fields_typed Sigma Gamma Delta
                          ((l, e) :: rest_e)
                          ((l, T) :: rest_t)
                          (effect_row_union eff1 eff2).

(** ** Notation for typing judgment *)

Notation "Sigma ';;' Gamma ';;' Delta '⊢' e '∷' T '/' eff" :=
  (has_type Sigma Gamma Delta e T eff) (at level 40).

(** ** A closed, well-typed term *)

Definition closed_well_typed (Sigma : effect_context) (e : expr) (T : ty) (eff : effect_row) : Prop :=
  has_type Sigma [] [] e T eff.
