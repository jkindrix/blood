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
  | EC_Annot : eval_context -> ty -> eval_context          (** (E : T) *)
  | EC_PerformArg :
      effect_name -> op_name ->
      eval_context -> eval_context                      (** perform E.op(E) *)
  | EC_Handle : handler -> eval_context -> eval_context (** with h handle E *)
  | EC_ExtendVal :
      label -> eval_context -> expr -> eval_context       (** {l = E | e} *)
  | EC_ExtendRec :
      label -> value -> eval_context -> eval_context      (** {l = v | E} *)
  | EC_Resume : eval_context -> eval_context              (** resume(E) *)
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
  | DC_Annot : delimited_context -> ty -> delimited_context
  | DC_PerformArg :
      effect_name -> op_name ->
      delimited_context -> delimited_context
  | DC_RecordField :
      list (label * expr) -> label ->
      delimited_context -> list (label * expr) -> delimited_context
  | DC_HandleOther :
      handler -> delimited_context -> delimited_context
  | DC_ExtendVal :
      label -> delimited_context -> expr -> delimited_context
  | DC_ExtendRec :
      label -> value -> delimited_context -> delimited_context
  | DC_Resume : delimited_context -> delimited_context
  .
  (** Note: DC_HandleOther allows performs to escape through
      a handler that does NOT handle the performed effect. *)

(** ** Plug expression into context *)

Fixpoint plug_eval (E : eval_context) (e : expr) : expr :=
  match E with
  | EC_Hole => e
  | EC_AppFun E' e2 => E_App (plug_eval E' e) e2
  | EC_AppArg v E' => E_App (value_to_expr v) (plug_eval E' e)
  | EC_Let E' e2 => E_Let (plug_eval E' e) e2
  | EC_Annot E' T => E_Annot (plug_eval E' e) T
  | EC_Select E' l => E_Select (plug_eval E' e) l
  | EC_PerformArg eff op E' => E_Perform eff op (plug_eval E' e)
  | EC_Handle h E' => E_Handle h (plug_eval E' e)
  | EC_ExtendVal l E' e2 => E_Extend l (plug_eval E' e) e2
  | EC_ExtendRec l v E' => E_Extend l (value_to_expr v) (plug_eval E' e)
  | EC_Resume E' => E_Resume (plug_eval E' e)
  end.

Fixpoint plug_delimited (D : delimited_context) (e : expr) : expr :=
  match D with
  | DC_Hole => e
  | DC_AppFun D' e2 => E_App (plug_delimited D' e) e2
  | DC_AppArg v D' => E_App (value_to_expr v) (plug_delimited D' e)
  | DC_Let D' e2 => E_Let (plug_delimited D' e) e2
  | DC_Annot D' T => E_Annot (plug_delimited D' e) T
  | DC_Select D' l => E_Select (plug_delimited D' e) l
  | DC_PerformArg eff op D' => E_Perform eff op (plug_delimited D' e)
  | DC_RecordField done l D' rest =>
      E_Record (done ++ (l, plug_delimited D' e) :: rest)
  | DC_HandleOther h D' =>
      E_Handle h (plug_delimited D' e)
  | DC_ExtendVal l D' e2 =>
      E_Extend l (plug_delimited D' e) e2
  | DC_ExtendRec l v D' =>
      E_Extend l (value_to_expr v) (plug_delimited D' e)
  | DC_Resume D' =>
      E_Resume (plug_delimited D' e)
  end.

(** ** dc_no_match: no handler in the delimited context handles the effect *)

Fixpoint dc_no_match (D : delimited_context) (eff_nm : effect_name) : Prop :=
  match D with
  | DC_Hole => True
  | DC_AppFun D' _ => dc_no_match D' eff_nm
  | DC_AppArg _ D' => dc_no_match D' eff_nm
  | DC_Let D' _ => dc_no_match D' eff_nm
  | DC_Select D' _ => dc_no_match D' eff_nm
  | DC_Annot D' _ => dc_no_match D' eff_nm
  | DC_PerformArg _ _ D' => dc_no_match D' eff_nm
  | DC_RecordField _ _ D' _ => dc_no_match D' eff_nm
  | DC_HandleOther h D' =>
      (match h with Handler _ _ clauses =>
         forall cl, In cl clauses ->
           match cl with OpClause en _ _ => en <> eff_nm end
       end) /\
      dc_no_match D' eff_nm
  | DC_ExtendVal _ D' _ => dc_no_match D' eff_nm
  | DC_ExtendRec _ _ D' => dc_no_match D' eff_nm
  | DC_Resume D' => dc_no_match D' eff_nm
  end.

(** ** Extract generation references from a delimited context

    GenRefs : Context → GenSnapshot
    Corresponds to FORMAL_SEMANTICS.md §4.2 *)

(** Collect generation references from values embedded in a
    delimited context (specifically from V_Continuation snapshots). *)

Fixpoint value_gen_refs (v : value) : list gen_ref :=
  match v with
  | V_Const _ => []
  | V_Lam _ _ => []
  | V_Record fields =>
      (fix fields_refs (fs : list (label * value)) : list gen_ref :=
        match fs with
        | [] => []
        | (_, fv) :: rest => value_gen_refs fv ++ fields_refs rest
        end) fields
  | V_Continuation _ _ (GenSnapshot refs) => refs
  end.

Fixpoint extract_gen_refs (D : delimited_context) : gen_snapshot :=
  match D with
  | DC_Hole => GenSnapshot []
  | DC_AppFun D' _ => extract_gen_refs D'
  | DC_AppArg v D' =>
      match extract_gen_refs D' with
      | GenSnapshot refs => GenSnapshot (value_gen_refs v ++ refs)
      end
  | DC_Let D' _ => extract_gen_refs D'
  | DC_Select D' _ => extract_gen_refs D'
  | DC_Annot D' _ => extract_gen_refs D'
  | DC_PerformArg _ _ D' => extract_gen_refs D'
  | DC_RecordField _ _ D' _ => extract_gen_refs D'
  | DC_HandleOther _ D' => extract_gen_refs D'
  | DC_ExtendVal _ D' _ => extract_gen_refs D'
  | DC_ExtendRec _ v D' =>
      match extract_gen_refs D' with
      | GenSnapshot refs => GenSnapshot (value_gen_refs v ++ refs)
      end
  | DC_Resume D' => extract_gen_refs D'
  end.

(** ** Configuration: expression + memory state *)

Record config := mk_config {
  cfg_expr : expr;
  cfg_mem  : memory;
}.

(** ** Deterministic field lookup for expressions

    Mirrors lookup_field from Typing.v but on (label * expr) lists. *)

Fixpoint find_field (fields : list (label * expr)) (l : label) : option expr :=
  match fields with
  | [] => None
  | (l', e) :: rest =>
      if label_eqb l' l then Some e
      else find_field rest l
  end.

(** ** Small-step reduction

    Corresponds to FORMAL_SEMANTICS.md §3 *)

Inductive step (Sigma : effect_context) : config -> config -> Prop :=

  (** [β-App]
      (λx:T. e) v  ──►  e[v/x] *)
  | Step_Beta : forall M T body v,
      step Sigma (mk_config (E_App (E_Lam T body) (value_to_expr v)) M)
           (mk_config (subst 0 (value_to_expr v) body) M)

  (** [β-Let]
      let x = v in e  ──►  e[v/x] *)
  | Step_Let : forall M v e2,
      is_value v = true ->
      step Sigma (mk_config (E_Let v e2) M)
           (mk_config (subst 0 v e2) M)

  (** [Record-Select]
      {l₁=v₁,...,lₙ=vₙ}.lᵢ  ──►  vᵢ

      Uses deterministic first-match lookup to align with the
      type system's lookup_field semantics. *)
  | Step_Select : forall M fields l e,
      find_field fields l = Some e ->
      (** All fields are values *)
      forallb (fun '(_, ei) => is_value ei) fields = true ->
      step Sigma (mk_config (E_Select (E_Record fields) l) M)
           (mk_config e M)

  (** [Record-Extend]
      {l = v | {l₁=v₁,...}} ──► {l=v, l₁=v₁,...} *)
  | Step_Extend : forall M l v fields,
      is_value v = true ->
      forallb (fun '(_, ei) => is_value ei) fields = true ->
      step Sigma (mk_config (E_Extend l v (E_Record fields)) M)
           (mk_config (E_Record ((l, v) :: fields)) M)

  (** [Annot]
      (v : T) ──► v *)
  | Step_Annot : forall M v T,
      is_value v = true ->
      step Sigma (mk_config (E_Annot v T) M)
           (mk_config v M)

  (** [Handle-Return]
      with h handle v  ──►  e_ret[v/x] *)
  | Step_HandleReturn : forall M hk e_ret clauses v,
      is_value v = true ->
      step Sigma (mk_config
              (E_Handle (Handler hk e_ret clauses) v) M)
           (mk_config
              (subst 0 v e_ret) M)

  (** [Handle-Op-Deep]
      with h handle D[perform E.op(v)]
        ──►  e_op[v/x, (λy. with h handle D[y])/resume]

      where h is a deep handler for effect E.
      The ret_ty is constrained by the operation's return type
      in Sigma, ensuring the continuation is well-typed. *)
  | Step_HandleOpDeep : forall M e_ret clauses D
                               eff_name op_nm v e_body
                               arg_ty ret_ty eff_sig snap,
      is_value v = true ->
      (** Find matching clause *)
      In (OpClause eff_name op_nm e_body) clauses ->
      (** Operation typing: constrains ret_ty *)
      lookup_effect Sigma eff_name = Some eff_sig ->
      lookup_op eff_sig op_nm = Some (arg_ty, ret_ty) ->
      (** Capture snapshot *)
      snap = extract_gen_refs D ->
      (** Build continuation: λy. with h handle D[y] *)
      let h := Handler Deep e_ret clauses in
      let kont := E_Lam ret_ty
                        (E_Handle h (plug_delimited D (E_Var 0))) in
      step Sigma (mk_config
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
                                  arg_ty ret_ty eff_sig snap,
      is_value v = true ->
      In (OpClause eff_name op_nm e_body) clauses ->
      (** Operation typing: constrains ret_ty *)
      lookup_effect Sigma eff_name = Some eff_sig ->
      lookup_op eff_sig op_nm = Some (arg_ty, ret_ty) ->
      snap = extract_gen_refs D ->
      let kont := E_Lam ret_ty
                        (plug_delimited D (E_Var 0)) in
      step Sigma (mk_config
              (E_Handle (Handler Shallow e_ret clauses)
                        (plug_delimited D (E_Perform eff_name op_nm v))) M)
           (mk_config
              (subst 0 v (subst 1 kont e_body)) M)

  (** [RecordEval]
      e ──► e'
      ─────────────────────────────────────────────
      {done, l=e, rest} ──► {done, l=e', rest} *)
  | Step_RecordEval : forall M M' done l e e' rest,
      step Sigma (mk_config e M) (mk_config e' M') ->
      step Sigma (mk_config (E_Record (done ++ (l, e) :: rest)) M)
           (mk_config (E_Record (done ++ (l, e') :: rest)) M')

  (** [Context]
      e ──► e'
      ─────────────
      E[e] ──► E[e'] *)
  | Step_Context : forall M M' E e e',
      step Sigma (mk_config e M) (mk_config e' M') ->
      step Sigma (mk_config (plug_eval E e) M)
           (mk_config (plug_eval E e') M')

  (** [Resume]
      resume(v)  ──►  v

      E_Resume is a transparent syntactic wrapper marking continuation
      call sites. The actual continuation invocation happens via E_App
      on the lambda bound in handler clause bodies. *)
  | Step_Resume : forall M v,
      is_value v = true ->
      step Sigma (mk_config (E_Resume v) M)
           (mk_config v M)
.

(** Make Sigma implicit in step constructors *)
Arguments Step_Beta {Sigma}.
Arguments Step_Let {Sigma}.
Arguments Step_Select {Sigma}.
Arguments Step_Extend {Sigma}.
Arguments Step_Annot {Sigma}.
Arguments Step_HandleReturn {Sigma}.
Arguments Step_HandleOpDeep {Sigma}.
Arguments Step_HandleOpShallow {Sigma}.
Arguments Step_RecordEval {Sigma}.
Arguments Step_Context {Sigma}.
Arguments Step_Resume {Sigma}.

(** ** Multi-step reduction (reflexive-transitive closure) *)

Inductive multi_step (Sigma : effect_context) : config -> config -> Prop :=
  | Multi_Refl : forall c,
      multi_step Sigma c c
  | Multi_Step : forall c1 c2 c3,
      step Sigma c1 c2 ->
      multi_step Sigma c2 c3 ->
      multi_step Sigma c1 c3.

Notation "c1 '──►*' c2" := (multi_step _ c1 c2) (at level 40).

(** ** A term is stuck if it's not a value and cannot step *)

Definition stuck (Sigma : effect_context) (c : config) : Prop :=
  ~ (is_value (cfg_expr c) = true) /\
  ~ (exists c', step Sigma c c') /\
  ~ (exists eff op v D,
       cfg_expr c = plug_delimited D (E_Perform eff op (value_to_expr v)) /\
       dc_no_match D eff).
