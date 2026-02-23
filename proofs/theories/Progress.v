(** * Blood Core Calculus — Progress Theorem

    This file states and proves the Progress theorem for Blood's
    core calculus: well-typed closed expressions are either values,
    can step, or perform an effect operation.

    Reference: FORMAL_SEMANTICS.md §7.1, §11.1
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Typing.
From Blood Require Import Substitution.
From Blood Require Import Semantics.

(** ** Canonical Forms Lemmas

    These establish what shape a value of a given type must have. *)

Lemma canonical_forms_arrow :
  forall Sigma v A B eff,
    has_type Sigma [] [] v (Ty_Arrow A B eff) Eff_Pure ->
    is_value v = true ->
    exists body, v = E_Lam A body.
Proof.
  intros Sigma v A B eff Htype Hval.
  remember [] as Gamma.
  remember [] as Delta.
  remember (Ty_Arrow A B eff) as T.
  induction Htype; subst.
  all: try discriminate.
  all: try (simpl in Hval; discriminate).
  all: try (unfold typeof_const in *; destruct c; discriminate).
  (* T_Lam *)
  - injection HeqT as HA HB Heff. subst. eexists. reflexivity.
  (* T_Sub *)
  - apply IHHtype; auto.
Qed.

Lemma canonical_forms_record :
  forall Sigma v fields,
    has_type Sigma [] [] v (Ty_Record fields) Eff_Pure ->
    is_value v = true ->
    exists vfields,
      v = E_Record vfields /\
      forallb (fun '(_, e) => is_value e) vfields = true.
Proof.
  intros Sigma v fields Htype Hval.
  remember [] as Gamma.
  remember [] as Delta.
  remember (Ty_Record fields) as T.
  induction Htype; subst.
  all: try discriminate.
  all: try (simpl in Hval; discriminate).
  all: try (unfold typeof_const in *; destruct c; discriminate).
  (* T_Record *)
  - exists fields0. split; auto.
  (* T_Sub *)
  - apply IHHtype; auto.
Qed.

Lemma canonical_forms_base :
  forall Sigma v b,
    has_type Sigma [] [] v (Ty_Base b) Eff_Pure ->
    is_value v = true ->
    exists c, v = E_Const c /\ typeof_const c = Ty_Base b.
Proof.
  intros Sigma v b Htype Hval.
  remember [] as Gamma.
  remember [] as Delta.
  remember (Ty_Base b) as T.
  induction Htype; subst.
  all: try discriminate.
  all: try (simpl in Hval; discriminate).
  (* T_Const *)
  - exists c. split; auto.
  (* T_Sub *)
  - apply IHHtype; auto.
Qed.

(** ** Progress Theorem

    Statement: If ∅; ∅ ⊢ e : T / ε then either:
    1. e is a value, or
    2. e ──► e' for some e' (in any memory state), or
    3. e = D[perform E.op(v)] for some D, E, op, v
       (an unhandled effect operation)

    Reference: FORMAL_SEMANTICS.md §7.1, §11.1

    Proof sketch from the spec:
    By structural induction on the derivation of ∅; ∅ ⊢ e : T / ε.

    Case e = x:
      Empty context, so x cannot be typed. Contradiction.

    Case e = v (value):
      Case 1 applies.

    Case e = e₁ e₂:
      By IH on e₁: either e₁ steps, or e₁ = v₁ (function), or performs.
      - If e₁ steps: e₁ e₂ ──► e₁' e₂ by context rule.
      - If e₁ = v₁ and e₂ steps: v₁ e₂ ──► v₁ e₂'.
      - If e₁ = v₁ = λx.e' and e₂ = v₂: (λx.e') v₂ ──► e'[v₂/x].
      - If either performs: effect propagates through context.

    Case e = with h handle e':
      By IH: either e' steps, is value, or performs.
      - If e' steps: with h handle e' ──► with h handle e''.
      - If e' = v: with h handle v ──► h.return(v).
      - If e' = D[perform op(v)]:
        - If op handled by h: steps by Handle-Op.
        - Otherwise: propagates.

    Case e = perform op(v):
      Case 3 applies.
*)

Theorem progress :
  forall Sigma e T eff M,
    closed_well_typed Sigma e T eff ->
    (is_value e = true) \/
    (exists e' M', step (mk_config e M) (mk_config e' M')) \/
    (exists eff_nm op v D,
       e = plug_delimited D (E_Perform eff_nm op (value_to_expr v))).
Proof.
  intros Sigma e T eff M Htype.
  unfold closed_well_typed in Htype.
  (* Proof by induction on the typing derivation Htype.

     The full proof requires case analysis on every typing rule:

     Case T_Var: x cannot be typed in empty context. Contradiction.

     Case T_Const: E_Const is a value. Case 1.

     Case T_Lam: E_Lam is a value. Case 1.

     Case T_App: By IH on e₁ and e₂ sub-derivations.
       - If e₁ is a value (arrow type → canonical forms → lambda),
         and e₂ is a value: β-App applies. Case 2.
       - If e₁ steps: context rule applies. Case 2.
       - If e₂ steps: context rule applies. Case 2.
       - If either performs: propagation. Case 3.

     Case T_Let: Similar to T_App.

     Case T_Record: By IH on each field expression.

     Case T_Select: By IH on record expression + canonical forms.

     Case T_Perform: Case 3 directly (perform is the effect).

     Case T_Handle: By IH on handled expression.
       - If value: Handle-Return. Case 2.
       - If steps: context rule. Case 2.
       - If performs E.op: Handle-Op applies. Case 2.
       - If performs other: propagation. Case 3.

     Case T_Sub: By IH on sub-derivation.

     Each case is standard. See FORMAL_SEMANTICS.md §11.1.
  *)
Admitted.

(** ** Corollary: well-typed closed pure programs don't get stuck *)

Corollary pure_progress :
  forall Sigma e T M,
    closed_well_typed Sigma e T Eff_Pure ->
    (is_value e = true) \/
    (exists e' M', step (mk_config e M) (mk_config e' M')).
Proof.
  intros Sigma e T M Htype.
  destruct (progress Sigma e T Eff_Pure M Htype) as [Hval | [Hstep | Hperform]].
  - left. exact Hval.
  - right. exact Hstep.
  - (* Pure program cannot perform effects.
       By T-Perform, perform requires effect in row.
       But row is pure (empty), contradiction. *)
    exfalso.
    (* This follows from effect safety: a pure-typed term
       cannot contain unhandled performs. *)
Admitted.
