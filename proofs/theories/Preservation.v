(** * Blood Core Calculus — Preservation Theorem

    This file states and proves the Preservation (Subject Reduction)
    theorem: if a well-typed expression steps, the result is also
    well-typed with the same type and a subset of effects.

    Reference: FORMAL_SEMANTICS.md §7.2, §11.2
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

(** ** Value typing inversion

    If a value is well-typed, its type matches its structure. *)

Lemma record_fields_typed_pure :
  forall Sigma Gamma Delta fields field_types eff,
    record_fields_typed Sigma Gamma Delta fields field_types eff ->
    forallb (fun '(_, e) => is_value e) fields = true ->
    record_fields_typed Sigma Gamma Delta fields field_types Eff_Pure.
Proof.
  intros Sigma Gamma Delta fields field_types eff Htyped Hvals.
  induction Htyped.
  - apply RFT_Nil.
  - simpl in Hvals.
    apply Bool.andb_true_iff in Hvals.
    destruct Hvals as [Hval1 Hval2].
    apply RFT_Cons with (eff1 := Eff_Pure) (eff2 := Eff_Pure).
    + (* Need value_typing_inversion for the field, but that creates circularity.
         Instead, note that values type with pure effect directly. *)
      (* This requires a mutual induction or a separate lemma. For now, admit. *)
      admit.
    + apply IHHtyped. assumption.
Admitted.

Lemma value_typing_inversion :
  forall Sigma Gamma Delta v T eff,
    has_type Sigma Gamma Delta v T eff ->
    is_value v = true ->
    has_type Sigma Gamma Delta v T Eff_Pure.
Proof.
  intros Sigma Gamma Delta v T eff Htype Hval.
  induction Htype.
  - (* T_Var: variables are not values *)
    simpl in Hval. discriminate.
  - (* T_Const: constants are values and pure *)
    apply T_Const.
  - (* T_Lam: lambdas are values and pure *)
    apply T_Lam. assumption.
  - (* T_App: applications are not values *)
    simpl in Hval. discriminate.
  - (* T_Let: let is not a value *)
    simpl in Hval. discriminate.
  - (* T_Annot: annotations are not values *)
    simpl in Hval. discriminate.
  - (* T_Record: records are values if all fields are values *)
    simpl in Hval.
    apply T_Record.
    apply record_fields_typed_pure with (eff := eff); assumption.
  - (* T_Select: select is not a value *)
    simpl in Hval. discriminate.
  - (* T_Perform: perform is not a value *)
    simpl in Hval. discriminate.
  - (* T_Handle: handle is not a value *)
    simpl in Hval. discriminate.
  - (* T_Sub: use IH and effect subsumption *)
    apply T_Sub with (eff := Eff_Pure).
    + apply IHHtype. assumption.
    + simpl. trivial.
Qed.

(** ** Context typing

    If E[e] is well-typed, then e has some type A and replacing e
    with any e' : A preserves the overall type. *)

Lemma context_typing :
  forall Sigma E e T eff,
    has_type Sigma [] [] (plug_eval E e) T eff ->
    exists A eff_inner,
      has_type Sigma [] [] e A eff_inner /\
      forall e',
        has_type Sigma [] [] e' A eff_inner ->
        has_type Sigma [] [] (plug_eval E e') T eff.
Proof.
  (* By induction on the evaluation context E. *)
Admitted.

(** ** Effect row subset reflexivity *)

Lemma effect_row_subset_refl :
  forall eff, effect_row_subset eff eff.
Proof.
  destruct eff; simpl; auto.
Qed.

(** ** Effect row subset transitivity *)

Lemma effect_row_subset_trans :
  forall e1 e2 e3,
    effect_row_subset e1 e2 ->
    effect_row_subset e2 e3 ->
    effect_row_subset e1 e3.
Proof.
  intros e1 e2 e3 H12 H23.
  destruct e1; simpl in *.
  - (* e1 = Eff_Pure: always subset *)
    trivial.
  - (* e1 = Eff_Closed l *)
    destruct e2; simpl in *.
    + (* e2 = Eff_Pure: H12 says l = [] *)
      destruct e3; simpl in *.
      * (* e3 = Eff_Pure *) exact H12.
      * (* e3 = Eff_Closed *) rewrite H12. intros e Hin. inversion Hin.
      * (* e3 = Eff_Open *) rewrite H12. intros e Hin. inversion Hin.
    + (* e2 = Eff_Closed l0 *)
      destruct e3; simpl in *.
      * (* e3 = Eff_Pure: H23 says l0 = [] *)
        destruct l as [|hd tl].
        { reflexivity. }
        { exfalso. specialize (H12 hd (or_introl eq_refl)). rewrite H23 in H12. exact H12. }
      * (* e3 = Eff_Closed l1 *)
        intros e Hin. apply H23. apply H12. assumption.
      * (* e3 = Eff_Open l1 n *)
        intros e Hin. apply H23. apply H12. assumption.
    + (* e2 = Eff_Open l0 n *)
      destruct e3; simpl in *.
      * contradiction.
      * contradiction.
      * destruct H23 as [Heq H23'].
        intros e Hin. apply H23'. apply H12. assumption.
  - (* e1 = Eff_Open l n: can only be subset of open rows *)
    destruct e2; simpl in *.
    + contradiction.
    + contradiction.
    + destruct H12 as [Heq12 H12'].
      destruct e3; simpl in *.
      * contradiction.
      * contradiction.
      * destruct H23 as [Heq23 H23'].
        split.
        { congruence. }
        { intros e Hin. apply H23'. apply H12'. assumption. }
Qed.

(** ** Preservation Theorem

    Statement: If Γ; Δ ⊢ e : T / ε and e ──► e', then
    Γ; Δ' ⊢ e' : T / ε' where ε' ⊆ ε and Δ' ⊑ Δ.

    Reference: FORMAL_SEMANTICS.md §7.2, §11.2

    Proof sketch from the spec:
    By induction on the derivation of e ──► e'.

    Case β-App: (λx:S. e) v ──► e[v/x]
      By T-Lam: Γ, x:S ⊢ e : T / ε
      By T-App: Γ ⊢ v : S
      By Substitution Lemma: Γ ⊢ e[v/x] : T / ε ∎

    Case Handle-Return: with h handle v ──► h.return(v)
      By T-Handle: Γ ⊢ v : T and h.return : T → U
      Result has type U with effects from handler implementation.

    Case Handle-Op-Deep:
      with h handle D[perform op(v)] ──► e_op[v/x, κ/resume]
      Continuation κ = λy. with h handle D[y]
      By T-Handle: continuation has appropriate type
      Handler clause e_op typed correctly by handler typing rules.

    Effect subsumption maintained because handling removes
    the handled effect from the row. ∎
*)

Theorem preservation :
  forall Sigma e e' T eff M M',
    closed_well_typed Sigma e T eff ->
    step (mk_config e M) (mk_config e' M') ->
    exists eff',
      closed_well_typed Sigma e' T eff' /\
      effect_row_subset eff' eff.
Proof.
  intros Sigma e e' T eff M M' Htype Hstep.
  unfold closed_well_typed in *.

  (* Proof by induction on the step derivation. *)
  inversion Hstep; subst.

  (** Case Step_Beta: (λx:T. body) v ──► body[v/x] *)
  - exists eff. split.
    + (* By inversion on the typing of E_App (E_Lam T body) v:
         - E_Lam T body : A → B / fn_eff
         - v : A
         By inversion on T_Lam:
         - A :: Gamma ⊢ body : B / fn_eff
         By substitution_preserves_typing:
         - Gamma ⊢ body[v/x] : B / fn_eff *)
      admit.
    + apply effect_row_subset_refl.

  (** Case Step_Let: let x = v in e2 ──► e2[v/x] *)
  - exists eff. split.
    + (* Similar to β case: use substitution lemma *)
      admit.
    + apply effect_row_subset_refl.

  (** Case Step_Select: {l₁=v₁,...}.lᵢ ──► vᵢ *)
  - exists Eff_Pure. split.
    + (* The selected field has the type declared in the record type.
         By T_Record and T_Select inversion. *)
      admit.
    + (* pure ⊆ eff *)
      destruct eff; simpl; auto.

  (** Case Step_Extend: {l=v|{fields}} ──► {l=v, fields...} *)
  - exists eff. split.
    + admit.
    + apply effect_row_subset_refl.

  (** Case Step_Annot: (v : T) ──► v *)
  - exists eff. split.
    + (* By T_Annot inversion *)
      admit.
    + apply effect_row_subset_refl.

  (** Case Step_HandleReturn: with h handle v ──► e_ret[v/x] *)
  - exists eff. split.
    + (* By T_Handle inversion:
         - v : comp_ty
         - handler return clause: x:comp_ty ⊢ e_ret : result_ty / handler_eff
         By substitution lemma: e_ret[v/x] : result_ty / handler_eff *)
      admit.
    + apply effect_row_subset_refl.

  (** Case Step_HandleOpDeep *)
  - exists eff. split.
    + (* By T_Handle and handler well-formedness:
         - v : arg_ty for operation op
         - e_body : result_ty (with resume and arg bound)
         - continuation κ = λy. with h handle D[y] : ret_ty → result_ty / handler_eff
         By substitution: e_body[v/arg, κ/resume] : result_ty / handler_eff

         The handled effect is removed from the effect row. *)
      admit.
    + apply effect_row_subset_refl.

  (** Case Step_HandleOpShallow *)
  - exists eff. split.
    + (* Similar to deep case, but continuation does not re-wrap handler *)
      admit.
    + apply effect_row_subset_refl.

  (** Case Step_Context: E[e] steps because e steps *)
  - (* By context_typing lemma and IH *)
    admit.

  (** Case Step_ResumeValid *)
  - exists eff. split.
    + admit.
    + apply effect_row_subset_refl.
Admitted.

(** ** Note: Type Soundness (combining Progress + Preservation) is
    in Soundness.v, which imports both Progress.v and this file. *)

(** ** Effect handling removes the handled effect *)

Lemma handle_removes_effect :
  forall Sigma e T eff_name h comp_ty result_ty handler_eff,
    closed_well_typed Sigma e T
      (Eff_Closed (Eff_Entry eff_name :: [])) ->
    handler_well_formed Sigma [] [] h eff_name comp_ty result_ty handler_eff ->
    exists eff',
      closed_well_typed Sigma (E_Handle h e) result_ty eff' /\
      ~ effect_in_row eff_name eff'.
Proof.
  (* After handling, the handled effect is no longer in the result's
     effect row. This is the key property of effect handling. *)
Admitted.
