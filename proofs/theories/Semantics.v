(** * Blood Core Calculus — Small-Step Semantics

    This file defines the small-step operational semantics for Blood's
    core calculus, including standard reduction, effect handling, and
    generation snapshot semantics.

    Reference: FORMAL_SEMANTICS.md §3 (Reduction Rules)
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
From Stdlib Require Import PeanoNat.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Typing.
From Blood Require Import Substitution.

(** ** Memory model

    Corresponds to FORMAL_SEMANTICS.md §4.4 *)

Record memory_cell := mk_cell {
  cell_value : option value;
  cell_gen   : nat;
}.

Definition memory := nat -> memory_cell.

(** ** Empty memory *)

Definition empty_memory : memory :=
  fun _ => mk_cell None 0.

(** ** Memory update *)

Definition mem_update (M : memory) (addr : nat) (c : memory_cell) : memory :=
  fun a => if a =? addr then c else M a.

(** ** Current generation query *)

Definition current_gen (M : memory) (addr : nat) : nat :=
  cell_gen (M addr).

(** ** Snapshot validation

    Valid(Γ_gen, M) ≡ ∀(a, g) ∈ Γ_gen. M(a).gen = g *)

Definition validate_snapshot (M : memory) (snap : gen_snapshot) : Prop :=
  match snap with
  | GenSnapshot refs =>
      Forall (fun gr =>
        match gr with
        | GenRef addr gen => current_gen M addr = gen
        end) refs
  end.

Definition validate_snapshot_dec (M : memory) (snap : gen_snapshot) : bool :=
  match snap with
  | GenSnapshot refs =>
      forallb (fun gr =>
        match gr with
        | GenRef addr gen => current_gen M addr =? gen
        end) refs
  end.

(** ** Evaluation contexts

    Corresponds to FORMAL_SEMANTICS.md §2.1

    E ::= □ | E e | v E | let x = E in e | ... *)

Inductive eval_context : Type :=
  | EC_Hole : eval_context                          (** □ *)
  | EC_AppFun : eval_context -> expr -> eval_context    (** E e *)
  | EC_AppArg : value -> eval_context -> eval_context   (** v E *)
  | EC_Let : eval_context -> expr -> eval_context       (** let x = E in e *)
  | EC_Select : eval_context -> label -> eval_context   (** E.l *)
  | EC_PerformArg :
      effect_name -> op_name ->
      eval_context -> eval_context                      (** perform E.op(E) *)
  | EC_Handle : handler -> eval_context -> eval_context (** with h handle E *)
  .

(** ** Delimited evaluation contexts

    Delimited contexts do NOT cross handler boundaries.
    Corresponds to FORMAL_SEMANTICS.md §2.2 *)

Inductive delimited_context : Type :=
  | DC_Hole : delimited_context
  | DC_AppFun : delimited_context -> expr -> delimited_context
  | DC_AppArg : value -> delimited_context -> delimited_context
  | DC_Let : delimited_context -> expr -> delimited_context
  | DC_Select : delimited_context -> label -> delimited_context
  | DC_PerformArg :
      effect_name -> op_name ->
      delimited_context -> delimited_context
  .
  (** Note: NO DC_Handle — that's the key difference *)

(** ** Plug expression into context *)

Fixpoint plug_eval (E : eval_context) (e : expr) : expr :=
  match E with
  | EC_Hole => e
  | EC_AppFun E' e2 => E_App (plug_eval E' e) e2
  | EC_AppArg v E' => E_App (value_to_expr v) (plug_eval E' e)
  | EC_Let E' e2 => E_Let (plug_eval E' e) e2
  | EC_Select E' l => E_Select (plug_eval E' e) l
  | EC_PerformArg eff op E' => E_Perform eff op (plug_eval E' e)
  | EC_Handle h E' => E_Handle h (plug_eval E' e)
  end.

Fixpoint plug_delimited (D : delimited_context) (e : expr) : expr :=
  match D with
  | DC_Hole => e
  | DC_AppFun D' e2 => E_App (plug_delimited D' e) e2
  | DC_AppArg v D' => E_App (value_to_expr v) (plug_delimited D' e)
  | DC_Let D' e2 => E_Let (plug_delimited D' e) e2
  | DC_Select D' l => E_Select (plug_delimited D' e) l
  | DC_PerformArg eff op D' => E_Perform eff op (plug_delimited D' e)
  end.

(** ** Extract generation references from a delimited context

    GenRefs : Context → GenSnapshot
    Corresponds to FORMAL_SEMANTICS.md §4.2 *)

(** For simplicity, we define this as an abstract function.
    In a full development, this would walk the context structure. *)

Parameter extract_gen_refs : delimited_context -> gen_snapshot.

(** ** Configuration: expression + memory state *)

Record config := mk_config {
  cfg_expr : expr;
  cfg_mem  : memory;
}.

(** ** Small-step reduction

    Corresponds to FORMAL_SEMANTICS.md §3 *)

Inductive step : config -> config -> Prop :=

  (** [β-App]
      (λx:T. e) v  ──►  e[v/x] *)
  | Step_Beta : forall M T body v,
      step (mk_config (E_App (E_Lam T body) (value_to_expr v)) M)
           (mk_config (subst 0 (value_to_expr v) body) M)

  (** [β-Let]
      let x = v in e  ──►  e[v/x] *)
  | Step_Let : forall M v e2,
      is_value v = true ->
      step (mk_config (E_Let v e2) M)
           (mk_config (subst 0 v e2) M)

  (** [Record-Select]
      {l₁=v₁,...,lₙ=vₙ}.lᵢ  ──►  vᵢ *)
  | Step_Select : forall M fields l e,
      In (l, e) fields ->
      is_value e = true ->
      (** All fields are values *)
      forallb (fun '(_, ei) => is_value ei) fields = true ->
      step (mk_config (E_Select (E_Record fields) l) M)
           (mk_config e M)

  (** [Record-Extend]
      {l = v | {l₁=v₁,...}} ──► {l=v, l₁=v₁,...} *)
  | Step_Extend : forall M l v fields,
      is_value v = true ->
      forallb (fun '(_, ei) => is_value ei) fields = true ->
      step (mk_config (E_Extend l v (E_Record fields)) M)
           (mk_config (E_Record ((l, v) :: fields)) M)

  (** [Annot]
      (v : T) ──► v *)
  | Step_Annot : forall M v T,
      is_value v = true ->
      step (mk_config (E_Annot v T) M)
           (mk_config v M)

  (** [Handle-Return]
      with h handle v  ──►  e_ret[v/x] *)
  | Step_HandleReturn : forall M hk e_ret clauses v,
      is_value v = true ->
      step (mk_config
              (E_Handle (Handler hk e_ret clauses) v) M)
           (mk_config
              (subst 0 v e_ret) M)

  (** [Handle-Op-Deep]
      with h handle D[perform E.op(v)]
        ──►  e_op[v/x, (λy. with h handle D[y])/resume]

      where h is a deep handler for effect E *)
  | Step_HandleOpDeep : forall M e_ret clauses D
                               eff_name op_nm v e_body
                               snap,
      is_value v = true ->
      (** Find matching clause *)
      In (OpClause eff_name op_nm e_body) clauses ->
      (** Capture snapshot *)
      snap = extract_gen_refs D ->
      (** Build continuation: λy. with h handle D[y] *)
      let h := Handler Deep e_ret clauses in
      let kont := E_Lam (Ty_Base TyUnit) (* placeholder type *)
                        (E_Handle h (plug_delimited D (E_Var 0))) in
      step (mk_config
              (E_Handle (Handler Deep e_ret clauses)
                        (plug_delimited D (E_Perform eff_name op_nm v))) M)
           (mk_config
              (** e_body[v/arg, kont/resume] *)
              (subst 0 v (subst 1 kont e_body)) M)

  (** [Handle-Op-Shallow]
      with h handle D[perform E.op(v)]
        ──►  e_op[v/x, (λy. D[y])/resume]

      Note: handler NOT re-wrapped around continuation *)
  | Step_HandleOpShallow : forall M e_ret clauses D
                                  eff_name op_nm v e_body
                                  snap,
      is_value v = true ->
      In (OpClause eff_name op_nm e_body) clauses ->
      snap = extract_gen_refs D ->
      let kont := E_Lam (Ty_Base TyUnit)
                        (plug_delimited D (E_Var 0)) in
      step (mk_config
              (E_Handle (Handler Shallow e_ret clauses)
                        (plug_delimited D (E_Perform eff_name op_nm v))) M)
           (mk_config
              (subst 0 v (subst 1 kont e_body)) M)

  (** [Context]
      e ──► e'
      ─────────────
      E[e] ──► E[e'] *)
  | Step_Context : forall M M' E e e',
      step (mk_config e M) (mk_config e' M') ->
      step (mk_config (plug_eval E e) M)
           (mk_config (plug_eval E e') M')

  (** [Resume-Valid]
      resume((κ, Γ_gen, ∅), v) ──► κ(v)
      if ∀(a,g) ∈ Γ_gen. currentGen(a) = g *)
  | Step_ResumeValid : forall M kont_body snap v,
      is_value v = true ->
      validate_snapshot M snap ->
      step (mk_config
              (E_App (E_Const (Const_Unit)) v)  (** simplified resume *)
              M)
           (mk_config
              (subst 0 v kont_body) M)
      (** Note: This is simplified. A full formalization would track
          continuations as first-class values with snapshots. *)
.

(** ** Multi-step reduction (reflexive-transitive closure) *)

Inductive multi_step : config -> config -> Prop :=
  | Multi_Refl : forall c,
      multi_step c c
  | Multi_Step : forall c1 c2 c3,
      step c1 c2 ->
      multi_step c2 c3 ->
      multi_step c1 c3.

Notation "c1 '──►*' c2" := (multi_step c1 c2) (at level 40).

(** ** A term is stuck if it's not a value and cannot step *)

Definition stuck (c : config) : Prop :=
  ~ (is_value (cfg_expr c) = true) /\
  ~ (exists c', step c c') /\
  ~ (exists eff op v D,
       cfg_expr c = plug_delimited D (E_Perform eff op (value_to_expr v))).
