(** * Blood — Linear Typing Judgment

    Defines a strengthened typing judgment [has_type_lin] that enforces
    linearity constraints, then proves [has_type_lin → has_type].

    This two-judgment design avoids modifying the existing [has_type] rules,
    preventing cascading breakage in Progress, Preservation, and Soundness.

    Reference: FORMAL_SEMANTICS.md §8 (Linear Types and Effects Interaction)
    Phase: M3 — Linearity

    Status: 0 Admitted.
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

(** ** Helper predicates for linearity enforcement *)

(** All linear bindings are marked used (no unconsumed linears).
    Used in TL_Const to ensure constants don't appear in contexts
    with live linear bindings. *)

Definition all_linear_consumed (Delta : lin_context) : Prop :=
  forall i,
    match nth_error Delta i with
    | Some (Lin_Linear, false) => False
    | _ => True
    end.

(** All linear bindings at positions other than [x] are marked used.
    Used in TL_Var to ensure the variable being used is the ONLY
    live linear binding. *)

Definition all_others_linear_consumed (Delta : lin_context) (x : var) : Prop :=
  forall i, i <> x ->
    match nth_error Delta i with
    | Some (Lin_Linear, false) => False
    | _ => True
    end.

(** Map type-level linearity annotations to Delta entries.
    [Ty_Linear T] → [Lin_Linear], [Ty_Affine T] → [Lin_Affine],
    everything else → [Lin_Unrestricted]. *)

Definition lin_of_type (T : ty) : linearity :=
  match T with
  | Ty_Linear _ => Lin_Linear
  | Ty_Affine _ => Lin_Affine
  | _ => Lin_Unrestricted
  end.

(** ** Free variable counting

    Count how many times variable [x] appears free in expression [e].
    Defined here (rather than LinearSafety.v) because the strengthened
    handler rule needs [count_var] in its premises. *)

Fixpoint count_var (x : var) (e : expr) : nat :=
  match e with
  | E_Var y => if x =? y then 1 else 0
  | E_Const _ => 0
  | E_Lam _ body => count_var (S x) body
  | E_App e1 e2 => count_var x e1 + count_var x e2
  | E_Let e1 e2 => count_var x e1 + count_var (S x) e2
  | E_Annot e1 _ => count_var x e1
  | E_Record fields =>
      fold_left (fun acc '(_, ei) => acc + count_var x ei) fields 0
  | E_Select e1 _ => count_var x e1
  | E_Extend _ e1 e2 => count_var x e1 + count_var x e2
  | E_Perform _ _ e1 => count_var x e1
  | E_Handle h e1 =>
      count_var x e1 +
      match h with
      | Handler _ e_ret clauses =>
          count_var (S x) e_ret +
          fold_left (fun acc cl =>
            match cl with
            | OpClause _ _ body => acc + count_var (S (S x)) body
            end) clauses 0
      end
  | E_Resume e1 => count_var x e1
  end.

Definition var_in (x : var) (e : expr) : Prop :=
  count_var x e > 0.

Definition linear_used_once (x : var) (e : expr) : Prop :=
  count_var x e = 1.

Definition affine_used_at_most_once (x : var) (e : expr) : Prop :=
  count_var x e <= 1.

(** No linear captures: if a binding is linear and unconsumed,
    it must not appear in the expression. *)

Definition no_linear_captures
    (Delta : lin_context) (clause_body : expr) : Prop :=
  forall x,
    nth_error Delta x = Some (Lin_Linear, false) ->
    count_var x clause_body = 0.

(** Multi-shot handler safety (propositional version, for existing defs) *)

Definition multishot_handler_safe
    (h : handler) (Delta : lin_context) : Prop :=
  match h with
  | Handler _ _ clauses =>
      forall cl,
        In cl clauses ->
        match cl with
        | OpClause _ _ body =>
            count_var 1 body > 1 ->
            no_linear_captures Delta body
        end
  end.

(** ** Strengthened typing judgment: has_type_lin

    Mirrors [has_type] but with additional premises at leaf rules
    and binder introductions to enforce linearity. *)

Inductive handler_well_formed_lin :
    effect_context -> type_context -> lin_context ->
    handler -> effect_name -> ty -> ty -> effect_row ->
    effect_row -> Prop :=
  (** Handler branches are mutually exclusive: either the computation
      returns (triggering e_ret) or an operation fires (triggering a clause).
      We split Delta between return and ops via [lin_split] so that each
      branch gets its own share of linear bindings. *)
  | HWF_Lin : forall Sigma Gamma Delta Delta_ret Delta_ops hk e_ret clauses
               eff_name comp_ty result_ty handler_effects comp_eff
               eff_sig,
      lookup_effect Sigma eff_name = Some eff_sig ->
      lin_split Delta Delta_ret Delta_ops ->
      has_type_lin Sigma (comp_ty :: Gamma) ((Lin_Unrestricted, false) :: Delta_ret)
               e_ret result_ty handler_effects ->
      op_clauses_well_formed_lin Sigma Gamma Delta_ops clauses
                             eff_name eff_sig
                             (match hk with Deep => result_ty | Shallow => comp_ty end)
                             (match hk with Deep => handler_effects | Shallow => comp_eff end)
                             result_ty handler_effects ->
      (forall op_nm arg_ty ret_ty,
         In (op_nm, arg_ty, ret_ty) eff_sig ->
         exists e_body, In (OpClause eff_name op_nm e_body) clauses) ->
      multishot_handler_safe_lin (Handler hk e_ret clauses) Delta ->
      handler_well_formed_lin Sigma Gamma Delta
                          (Handler hk e_ret clauses)
                          eff_name comp_ty result_ty handler_effects comp_eff

with op_clauses_well_formed_lin :
    effect_context -> type_context -> lin_context ->
    list op_clause -> effect_name -> effect_sig ->
    ty -> effect_row ->
    ty -> effect_row ->
    Prop :=
  | OpClauses_Nil_Lin : forall Sigma Gamma Delta eff_name sig
                           resume_ret_ty resume_eff result_ty eff,
      all_linear_consumed Delta ->
      op_clauses_well_formed_lin Sigma Gamma Delta [] eff_name sig
                             resume_ret_ty resume_eff result_ty eff
  (** Operation clauses are also mutually exclusive: at most one fires.
      We split Delta between the current clause body and the remaining
      clauses so each gets its own share of linear bindings. *)
  | OpClauses_Cons_Lin :
      forall Sigma Gamma Delta Delta_body Delta_rest eff_name op_nm e_body rest
             sig resume_ret_ty resume_eff result_ty handler_eff arg_ty ret_ty,
      lin_split Delta Delta_body Delta_rest ->
      lookup_op sig op_nm = Some (arg_ty, ret_ty) ->
      has_type_lin Sigma
               (arg_ty :: Ty_Arrow ret_ty resume_ret_ty resume_eff :: Gamma)
               ((lin_of_type arg_ty, false) :: (Lin_Unrestricted, false) :: Delta_body)
               e_body result_ty handler_eff ->
      op_clauses_well_formed_lin Sigma Gamma Delta_rest rest eff_name sig
                             resume_ret_ty resume_eff result_ty handler_eff ->
      op_clauses_well_formed_lin Sigma Gamma Delta
                             (OpClause eff_name op_nm e_body :: rest)
                             eff_name sig resume_ret_ty resume_eff
                             result_ty handler_eff

with has_type_lin :
    effect_context -> type_context -> lin_context ->
    expr -> ty -> effect_row -> Prop :=

  (** [TL-Var]
      x : T ∈ Γ    Δ(x) = (_, false)    all other linears consumed
      ──────────────────────────────────────────────────────────
      Γ; Δ ⊢_lin x : T / pure *)
  | TL_Var : forall Sigma Gamma Delta x T,
      lookup_var Gamma x = Some T ->
      (x < length Delta ->
       exists lm, nth_error Delta x = Some (lm, false)) ->
      all_others_linear_consumed Delta x ->
      has_type_lin Sigma Gamma Delta (E_Var x) T Eff_Pure

  (** [TL-Const]
      all linear bindings consumed
      ─────────────────────────────
      Γ; Δ ⊢_lin c : typeof(c) / pure *)
  | TL_Const : forall Sigma Gamma Delta c,
      all_linear_consumed Delta ->
      has_type_lin Sigma Gamma Delta (E_Const c) (typeof_const c) Eff_Pure

  (** [TL-Lam]
      Γ, x:A; Δ, x:lin_of_type(A) ⊢_lin e : B / ε
      ─────────────────────────────────────────────────
      Γ; Δ ⊢_lin λx:A. e : A → B / ε / pure *)
  | TL_Lam : forall Sigma Gamma Delta A B eff body,
      has_type_lin Sigma (A :: Gamma) ((lin_of_type A, false) :: Delta)
               body B eff ->
      has_type_lin Sigma Gamma Delta (E_Lam A body) (Ty_Arrow A B eff) Eff_Pure

  (** [TL-App]
      Δ = Δ₁ ⊗ Δ₂
      Γ; Δ₁ ⊢_lin e₁ : A → B / ε / ε₁    Γ; Δ₂ ⊢_lin e₂ : A / ε₂
      ────────────────────────────────────────────────────────────────
      Γ; Δ ⊢_lin e₁ e₂ : B / ε ∪ ε₁ ∪ ε₂ *)
  | TL_App : forall Sigma Gamma Delta Delta1 Delta2
                   e1 e2 A B fn_eff eff1 eff2,
      lin_split Delta Delta1 Delta2 ->
      has_type_lin Sigma Gamma Delta1 e1 (Ty_Arrow A B fn_eff) eff1 ->
      has_type_lin Sigma Gamma Delta2 e2 A eff2 ->
      has_type_lin Sigma Gamma Delta (E_App e1 e2) B
               (effect_row_union fn_eff (effect_row_union eff1 eff2))

  (** [TL-Let]
      Δ = Δ₁ ⊗ Δ₂
      Γ; Δ₁ ⊢_lin e₁ : A / ε₁    Γ, x:A; Δ₂, x:lin_of_type(A) ⊢_lin e₂ : B / ε₂
      ─────────────────────────────────────────────────────────────────────────────────
      Γ; Δ ⊢_lin let x = e₁ in e₂ : B / ε₁ ∪ ε₂ *)
  | TL_Let : forall Sigma Gamma Delta Delta1 Delta2
                   e1 e2 A B eff1 eff2,
      lin_split Delta Delta1 Delta2 ->
      has_type_lin Sigma Gamma Delta1 e1 A eff1 ->
      has_type_lin Sigma (A :: Gamma) ((lin_of_type A, false) :: Delta2)
               e2 B eff2 ->
      has_type_lin Sigma Gamma Delta (E_Let e1 e2) B
               (effect_row_union eff1 eff2)

  | TL_Annot : forall Sigma Gamma Delta e T eff,
      has_type_lin Sigma Gamma Delta e T eff ->
      has_type_lin Sigma Gamma Delta (E_Annot e T) T eff

  | TL_Record : forall Sigma Gamma Delta fields field_types eff,
      record_fields_typed_lin Sigma Gamma Delta fields field_types eff ->
      has_type_lin Sigma Gamma Delta (E_Record fields)
               (Ty_Record field_types) eff

  | TL_Select : forall Sigma Gamma Delta e l T fields eff,
      has_type_lin Sigma Gamma Delta e (Ty_Record fields) eff ->
      lookup_field fields l = Some T ->
      has_type_lin Sigma Gamma Delta (E_Select e l) T eff

  | TL_Perform : forall Sigma Gamma Delta e
                       eff_name op eff_sig arg_ty ret_ty eff',
      lookup_effect Sigma eff_name = Some eff_sig ->
      lookup_op eff_sig op = Some (arg_ty, ret_ty) ->
      has_type_lin Sigma Gamma Delta e arg_ty eff' ->
      has_type_lin Sigma Gamma Delta
               (E_Perform eff_name op e) ret_ty
               (effect_row_union
                  (Eff_Closed [Eff_Entry eff_name]) eff')

  | TL_Handle : forall Sigma Gamma Delta Delta1 Delta2
                      h e eff_name comp_ty result_ty
                      handler_eff comp_eff,
      lin_split Delta Delta1 Delta2 ->
      has_type_lin Sigma Gamma Delta1 e comp_ty comp_eff ->
      handler_well_formed_lin Sigma Gamma Delta2 h
                          eff_name comp_ty result_ty handler_eff comp_eff ->
      (forall en, effect_in_row en comp_eff ->
         en <> eff_name -> effect_in_row en handler_eff) ->
      has_type_lin Sigma Gamma Delta (E_Handle h e) result_ty handler_eff

  | TL_Sub : forall Sigma Gamma Delta e T eff eff',
      has_type_lin Sigma Gamma Delta e T eff ->
      effect_row_subset eff eff' ->
      has_type_lin Sigma Gamma Delta e T eff'

with record_fields_typed_lin :
    effect_context -> type_context -> lin_context ->
    list (label * expr) -> list (label * ty) -> effect_row -> Prop :=
  | RFT_Nil_Lin : forall Sigma Gamma Delta,
      all_linear_consumed Delta ->
      record_fields_typed_lin Sigma Gamma Delta [] [] Eff_Pure
  (** NEW: lin_split across record fields *)
  | RFT_Cons_Lin : forall Sigma Gamma Delta Delta1 Delta2
                          l e T rest_e rest_t eff1 eff2,
      lin_split Delta Delta1 Delta2 ->
      has_type_lin Sigma Gamma Delta1 e T eff1 ->
      record_fields_typed_lin Sigma Gamma Delta2 rest_e rest_t eff2 ->
      record_fields_typed_lin Sigma Gamma Delta
                          ((l, e) :: rest_e)
                          ((l, T) :: rest_t)
                          (effect_row_union eff1 eff2)

(** Multi-shot handler restriction for the strengthened judgment.
    Re-defined here to avoid circular dependency issues. *)

with multishot_handler_safe_lin : handler -> lin_context -> Prop :=
  | MHSL_Handler : forall hk e_ret clauses Delta,
      (forall cl,
         In cl clauses ->
         match cl with
         | OpClause _ _ body =>
             count_var 1 body > 1 ->
             no_linear_captures Delta body
         end) ->
      multishot_handler_safe_lin (Handler hk e_ret clauses) Delta.

(** ** Mutual induction scheme for has_type_lin *)

Scheme has_type_lin_mut_ind := Induction for has_type_lin Sort Prop
  with handler_wf_lin_mut_ind := Induction for handler_well_formed_lin Sort Prop
  with op_clauses_wf_lin_mut_ind := Induction for op_clauses_well_formed_lin Sort Prop
  with record_fields_typed_lin_mut_ind := Induction for record_fields_typed_lin Sort Prop
  with multishot_handler_safe_lin_mut_ind := Induction for multishot_handler_safe_lin Sort Prop.

(** Helper: record_fields_typed is irrelevant in Delta.
    Standalone version of the fact proved inside has_type_lin_irrelevant. *)

Lemma record_fields_typed_delta_irrelevant :
  forall Sigma Gamma Delta fields field_types eff,
    record_fields_typed Sigma Gamma Delta fields field_types eff ->
    forall Delta',
    record_fields_typed Sigma Gamma Delta' fields field_types eff.
Proof.
  intros Sigma Gamma Delta fields field_types eff H.
  induction H; intros Delta'.
  - apply RFT_Nil.
  - apply RFT_Cons.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ H).
    + apply IHrecord_fields_typed.
Qed.

(** ** Bridging lemma: has_type_lin → has_type

    Every linearity-checked derivation can be erased to a standard
    typing derivation by dropping the extra premises and using
    [has_type_lin_irrelevant] to adjust Delta at binder introductions. *)

Lemma has_type_lin_to_has_type :
  forall Sigma Gamma Delta e T eff,
    has_type_lin Sigma Gamma Delta e T eff ->
    has_type Sigma Gamma Delta e T eff.
Proof.
  apply (has_type_lin_mut_ind
    (fun Sigma Gamma Delta e T eff _ =>
       has_type Sigma Gamma Delta e T eff)
    (fun Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff comp_eff _ =>
       handler_well_formed Sigma Gamma Delta h
                           eff_name comp_ty result_ty handler_eff comp_eff)
    (fun Sigma Gamma Delta clauses eff_name eff_sig rrt re result_ty handler_eff _ =>
       op_clauses_well_formed Sigma Gamma Delta clauses
                              eff_name eff_sig rrt re result_ty handler_eff)
    (fun Sigma Gamma Delta fields field_types eff _ =>
       record_fields_typed Sigma Gamma Delta fields field_types eff)
    (fun h Delta _ => True)).

  - (* TL_Var *)
    intros Sigma Gamma Delta x T Hlook _ _.
    apply T_Var. exact Hlook.

  - (* TL_Const *)
    intros Sigma Gamma Delta c _.
    apply T_Const.

  - (* TL_Lam — need to convert (lin_of_type A, false) :: Delta to
       (Lin_Unrestricted, false) :: Delta *)
    intros Sigma Gamma Delta A B eff body _ IH.
    apply T_Lam.
    apply (has_type_lin_irrelevant _ _ _ _ _ _ IH).

  - (* TL_App *)
    intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B fn_eff eff1 eff2
           Hsplit _ IH1 _ IH2.
    eapply T_App; [exact Hsplit | exact IH1 | exact IH2].

  - (* TL_Let — need to convert (lin_of_type A, false) :: Delta2 to
       (Lin_Unrestricted, false) :: Delta2 *)
    intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B eff1 eff2
           Hsplit _ IH1 _ IH2.
    eapply T_Let.
    + exact Hsplit.
    + exact IH1.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ IH2).

  - (* TL_Annot *)
    intros Sigma Gamma Delta e T eff _ IH.
    apply T_Annot. exact IH.

  - (* TL_Record *)
    intros Sigma Gamma Delta fields field_types eff _ IH.
    apply T_Record. exact IH.

  - (* TL_Select *)
    intros Sigma Gamma Delta e l T fields eff _ IH Hlook.
    eapply T_Select; [exact IH | exact Hlook].

  - (* TL_Perform *)
    intros Sigma Gamma Delta e eff_name op eff_sig arg_ty ret_ty eff'
           Hlookeff Hlookop _ IH.
    eapply T_Perform; [exact Hlookeff | exact Hlookop | exact IH].

  - (* TL_Handle *)
    intros Sigma Gamma Delta Delta1 Delta2 h e eff_name comp_ty result_ty
           handler_eff comp_eff Hsplit _ IHe _ IHh Hpass.
    eapply T_Handle; [exact Hsplit | exact IHe | exact IHh | exact Hpass].

  - (* TL_Sub *)
    intros Sigma Gamma Delta e T eff eff' _ IH Hsub.
    eapply T_Sub; [exact IH | exact Hsub].

  - (* HWF_Lin — drop multishot premise, reunify split Delta *)
    intros Sigma Gamma Delta Delta_ret Delta_ops hk e_ret clauses eff_name
           comp_ty result_ty handler_eff comp_eff eff_sig
           Hlook Hsplit _ IHret _ IHclauses Hcov _ _.
    eapply HWF.
    + exact Hlook.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ IHret).
    + apply (op_clauses_wf_lin_irrelevant _ _ _ _ _ _ _ _ _ _ IHclauses).
    + exact Hcov.

  - (* OpClauses_Nil_Lin *)
    intros. apply OpClauses_Nil.

  - (* OpClauses_Cons_Lin — reunify split Delta *)
    intros Sigma Gamma Delta Delta_body Delta_rest eff_nm op_nm e_body rest
           sig rrt re result_ty handler_eff arg_ty ret_ty
           Hsplit Hlookop _ IHbody _ IHrest.
    eapply OpClauses_Cons.
    + exact Hlookop.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ IHbody).
    + apply (op_clauses_wf_lin_irrelevant _ _ _ _ _ _ _ _ _ _ IHrest).

  - (* RFT_Nil_Lin *)
    intros. apply RFT_Nil.

  - (* RFT_Cons_Lin — reunify split Delta via has_type_lin_irrelevant *)
    intros Sigma Gamma Delta Delta1 Delta2 l e T rest_e rest_t eff1 eff2
           Hsplit _ IH1 _ IH2.
    apply RFT_Cons.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ IH1).
    + apply (record_fields_typed_delta_irrelevant _ _ _ _ _ _ IH2).

  - (* MHSL_Handler — trivial *)
    intros. exact I.
Qed.

(** ** Summary

    LinearTyping.v defines the strengthened [has_type_lin] judgment that
    enforces linearity constraints at leaf rules:

    - TL_Var: checks Delta(x) is available and all other linears consumed
    - TL_Const: checks all linears consumed
    - TL_Lam/TL_Let: introduce [lin_of_type A] (not always Unrestricted)
    - RFT_Cons_Lin: splits Delta across record fields
    - HWF_Lin: requires [multishot_handler_safe_lin]
    - OpClauses_Cons_Lin: introduces [lin_of_type arg_ty] for arg

    The bridge lemma [has_type_lin_to_has_type] proves that every
    linearity-checked derivation is also a valid standard derivation,
    using [has_type_lin_irrelevant] from Substitution.v.

    Status: 0 Admitted.
*)
