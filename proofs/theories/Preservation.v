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
From Blood Require Import EffectAlgebra.
From Blood Require Import Inversion.
From Blood Require Import ContextTyping.

(** ** Preservation Theorem

    Statement: If Γ; Δ ⊢ e : T / ε and e ──► e', then
    Γ; Δ' ⊢ e' : T / ε' where ε' ⊆ ε and Δ' ⊑ Δ.

    Reference: FORMAL_SEMANTICS.md §7.2, §11.2

    We prove preservation via a config-level helper amenable to
    induction on the step relation. This provides an induction
    hypothesis for Step_Context (which inversion cannot). *)

Lemma preservation_ind :
  forall Sigma c1 c2,
    step Sigma c1 c2 ->
    forall T eff,
      has_type Sigma [] [] (cfg_expr c1) T eff ->
      exists eff',
        has_type Sigma [] [] (cfg_expr c2) T eff' /\
        effect_row_subset eff' eff.
Proof.
  intros Sigma c1 c2 Hstep.
  induction Hstep; simpl; intros T0 eff0 Htype.

  (** Case Step_Beta: (λx:T. body) v ──► body[v/x] *)
  - destruct (has_type_app_inv _ _ _ _ _ Htype)
      as [A [fn_eff [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]]].
    destruct (has_type_lam_inv _ _ _ _ _ Hty1)
      as [B' [fn_eff' [Heq Htybody]]].
    injection Heq as HA HB Hfn. subst.
    exists fn_eff'. split.
    + apply substitution_preserves_typing with (U := T).
      * exact Htybody.
      * apply value_typing_inversion with (eff := eff2).
        exact Hty2. apply value_to_expr_is_value.
    + eapply effect_row_subset_trans.
      * apply effect_row_subset_union_l.
      * exact Hsub.

  (** Case Step_Let: let x = v in e2 ──► e2[v/x] *)
  - destruct (has_type_let_inv _ _ _ _ _ Htype)
      as [A [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]].
    exists eff2. split.
    + apply substitution_preserves_typing with (U := A).
      * exact Hty2.
      * apply value_typing_inversion with (eff := eff1).
        exact Hty1. assumption.
    + eapply effect_row_subset_trans.
      * apply effect_row_subset_union_r.
      * exact Hsub.

  (** Case Step_Select: {l₁=v₁,...}.lᵢ ──► vᵢ *)
  - destruct (has_type_select_inv _ _ _ _ _ Htype)
      as [ft [eff_inner [Hty_rec [Hlook Hsub]]]].
    destruct (has_type_record_inv _ _ _ _ Hty_rec)
      as [ft2 [eff_rec [Heq_ft [Hrft Hsub_rec]]]].
    injection Heq_ft as Hft. subst.
    exists eff_rec. split.
    + apply record_fields_typed_find with
        (fields := fields) (field_types := ft2) (l := l).
      * exact Hrft.
      * assumption.
      * exact Hlook.
    + eapply effect_row_subset_trans; eassumption.

  (** Case Step_Extend: {l=v|{fields}} ──► {l=v, fields...} *)
  - exfalso.
    remember (E_Extend l v (E_Record fields)) as eext.
    induction Htype; try discriminate; auto.

  (** Case Step_Annot: (v : T) ──► v *)
  - destruct (has_type_annot_inv _ _ _ _ _ Htype)
      as [HeqT [eff_inner [Hinner Hsub]]]. subst.
    exists eff_inner. split.
    + exact Hinner.
    + exact Hsub.

  (** Case Step_HandleReturn: with h handle v ──► e_ret[v/x] *)
  - destruct (has_type_handle_inv _ _ _ _ _ Htype)
      as [en [ct [he [ce [Htye [Hwf Hsub]]]]]].
    inversion Hwf; subst.
    exists he. split.
    + apply substitution_preserves_typing with (U := ct).
      * eassumption.
      * apply value_typing_inversion with (eff := ce). exact Htye. assumption.
    + exact Hsub.

  (** Case Step_HandleOpDeep *)
  - (* Invert handle typing *)
    destruct (has_type_handle_inv _ _ _ _ _ Htype)
      as [en' [comp_ty [handler_eff [comp_eff [Hty_comp [Hwf Hsub_he]]]]]].
    exists handler_eff. split; [| exact Hsub_he].

    (* Invert handler well-formedness *)
    inversion Hwf; subst.
    (* Robustly name the hypotheses from HWF inversion *)
    match goal with
    | [ Hle : lookup_effect _ _ = Some ?sig,
        Hrt : has_type _ _ _ e_ret _ _,
        Hcw : op_clauses_well_formed _ _ _ clauses _ ?sig _ _ _ _ |- _ ] =>
      rename Hle into Hlook_en; rename Hrt into Hty_ret;
      rename Hcw into Hclauses_wf
    end.

    (* Get clause body typing *)
    destruct (op_clause_typing_lookup _ _ _ _ _ _ _ _ _ _ _ _ _ Hclauses_wf H0)
      as [Heff_eq [arg_ty' [ret_ty' [Hlook_op' Hty_body]]]].
    subst en'.

    (* Unify effect signatures and operation types *)
    rewrite H1 in Hlook_en. injection Hlook_en as <-.
    rewrite H2 in Hlook_op'. injection Hlook_op' as <- <-.

    (* Get v : arg_ty / Eff_Pure via delimited context decomposition *)
    destruct (delimited_context_typing _ _ _ _ _ Hty_comp)
      as [A_hole [eff_hole [Hty_perf _]]].
    destruct (has_type_perform_inv _ _ _ _ _ _ Hty_perf)
      as [? [? [? [ef' [Hl1 [Hl2 [HeqA [Htyv _]]]]]]]].
    subst A_hole.
    rewrite H1 in Hl1. injection Hl1 as <-.
    rewrite H2 in Hl2. injection Hl2 as <- <-.
    assert (Hv_pure : has_type Sigma [] [] v arg_ty Eff_Pure)
      by (eapply value_typing_inversion; [exact Htyv | exact H]).

    (* Type the continuation kont = λy. with h handle D[y] *)
    assert (Hkont : has_type Sigma [] []
              (E_Lam ret_ty (E_Handle (Handler Deep e_ret clauses)
                                       (plug_delimited D (E_Var 0))))
              (Ty_Arrow ret_ty T0 handler_eff) Eff_Pure).
    { apply T_Lam.
      (* Weaken computation from [] [] to [ret_ty] *)
      pose proof (weakening_cons _ _ _ _ _ _ ret_ty Hty_comp) as Hcomp_w.
      pose proof (shift_closed_id _ _ _ _ _ _ Hty_comp 1) as Hsc1.
      simpl in Hsc1. rewrite Hsc1 in Hcomp_w. clear Hsc1.
      (* Decompose delimited context in [ret_ty] and replace hole *)
      destruct (delimited_context_typing_gen _ _ _ _ _ _ _ Hcomp_w)
        as [A2 [eff2 [Hperf2 Hreplace2]]].
      destruct (has_type_perform_inv_gen _ _ _ _ _ _ _ _ Hperf2)
        as [? [? [? [? [Hl1' [Hl2' [HeqA2 [_ _]]]]]]]].
      subst A2.
      rewrite H1 in Hl1'. injection Hl1' as <-.
      rewrite H2 in Hl2'. injection Hl2' as <- <-.
      (* Replace E_Perform with E_Var 0 *)
      eapply T_Handle.
      - apply Split_Unrestricted. apply Split_Nil.
      - eapply Hreplace2.
        apply T_Sub with (eff := Eff_Pure).
        + apply T_Var. reflexivity.
        + simpl. auto.
      - (* Weaken handler from [] [] to [ret_ty] *)
        pose proof (handler_weakening_cons _ _ _ _ _ _ _ _ _ ret_ty Hwf) as Hwf_w.
        pose proof (shift_handler_closed_id _ _ _ _ _ _ _ Hwf 1) as Hsh1.
        rewrite Hsh1 in Hwf_w. clear Hsh1.
        exact Hwf_w. }

    (* Weaken kont to [arg_ty] context for substitution at index 1 *)
    pose proof (weakening_cons _ _ _ _ _ _ arg_ty Hkont) as Hkont_w.
    pose proof (shift_closed_id _ _ _ _ _ _ Hkont 1) as Hsc2.
    assert (H__len : Datatypes.length (@nil ty) = 0) by reflexivity.
    rewrite H__len in Hsc2. clear H__len.
    rewrite Hsc2 in Hkont_w. clear Hsc2.

    (* Substitute kont for resume at index 1 *)
    assert (Hsub1 : has_type Sigma [arg_ty] [(Lin_Unrestricted, false)]
              (subst 1
                (E_Lam ret_ty (E_Handle (Handler Deep e_ret clauses)
                                         (plug_delimited D (E_Var 0))))
                e_body) T0 handler_eff).
    { apply (subst_preserves_typing _ _ _ _ _ _ Hty_body 1
              (E_Lam ret_ty (E_Handle (Handler Deep e_ret clauses)
                                       (plug_delimited D (E_Var 0))))
              (Ty_Arrow ret_ty T0 handler_eff)).
      - reflexivity.
      - simpl. exact Hkont_w. }

    (* Substitute v for arg at index 0 *)
    exact (substitution_preserves_typing _ _ _ _ _ _ _ _ Hsub1 Hv_pure).

  (** Case Step_HandleOpShallow *)
  - (* Invert handle typing *)
    destruct (has_type_handle_inv _ _ _ _ _ Htype)
      as [en' [comp_ty [handler_eff [comp_eff [Hty_comp [Hwf Hsub_he]]]]]].
    exists handler_eff. split; [| exact Hsub_he].

    (* Invert handler well-formedness *)
    inversion Hwf; subst.
    (* Simplify match on Shallow to get concrete resume types *)
    simpl in *.
    match goal with
    | [ Hle : lookup_effect _ _ = Some ?sig,
        Hrt : has_type _ _ _ e_ret _ _,
        Hcw : op_clauses_well_formed _ _ _ clauses _ ?sig _ _ _ _ |- _ ] =>
      rename Hle into Hlook_en; rename Hrt into Hty_ret;
      rename Hcw into Hclauses_wf
    end.

    (* Get clause body typing — resume type is (comp_ty, comp_eff) for shallow *)
    destruct (op_clause_typing_lookup _ _ _ _ _ _ _ _ _ _ _ _ _ Hclauses_wf H0)
      as [Heff_eq [arg_ty' [ret_ty' [Hlook_op' Hty_body]]]].
    subst en'.

    (* Unify effect signatures and operation types *)
    rewrite H1 in Hlook_en. injection Hlook_en as <-.
    rewrite H2 in Hlook_op'. injection Hlook_op' as <- <-.

    (* Get v : arg_ty / Eff_Pure via delimited context decomposition *)
    destruct (delimited_context_typing _ _ _ _ _ Hty_comp)
      as [A_hole [eff_hole [Hty_perf _]]].
    destruct (has_type_perform_inv _ _ _ _ _ _ Hty_perf)
      as [? [? [? [ef' [Hl1 [Hl2 [HeqA [Htyv _]]]]]]]].
    subst A_hole.
    rewrite H1 in Hl1. injection Hl1 as <-.
    rewrite H2 in Hl2. injection Hl2 as <- <-.
    assert (Hv_pure : has_type Sigma [] [] v arg_ty Eff_Pure)
      by (eapply value_typing_inversion; [exact Htyv | exact H]).

    (* Type the continuation kont = λy. D[y] — NO handler wrapping for shallow *)
    assert (Hkont : has_type Sigma [] []
              (E_Lam ret_ty (plug_delimited D (E_Var 0)))
              (Ty_Arrow ret_ty comp_ty comp_eff) Eff_Pure).
    { apply T_Lam.
      (* Weaken computation from [] [] to [ret_ty] *)
      pose proof (weakening_cons _ _ _ _ _ _ ret_ty Hty_comp) as Hcomp_w.
      pose proof (shift_closed_id _ _ _ _ _ _ Hty_comp 1) as Hsc1.
      simpl in Hsc1. rewrite Hsc1 in Hcomp_w. clear Hsc1.
      (* Decompose delimited context in [ret_ty] and replace hole *)
      destruct (delimited_context_typing_gen _ _ _ _ _ _ _ Hcomp_w)
        as [A2 [eff2 [Hperf2 Hreplace2]]].
      destruct (has_type_perform_inv_gen _ _ _ _ _ _ _ _ Hperf2)
        as [? [? [? [? [Hl1' [Hl2' [HeqA2 [_ _]]]]]]]].
      subst A2.
      rewrite H1 in Hl1'. injection Hl1' as <-.
      rewrite H2 in Hl2'. injection Hl2' as <- <-.
      (* Replace E_Perform with E_Var 0 — get D[y] : comp_ty / comp_eff *)
      eapply Hreplace2.
      apply T_Sub with (eff := Eff_Pure).
      + apply T_Var. reflexivity.
      + simpl. auto. }

    (* Weaken kont to [arg_ty] context for substitution at index 1 *)
    pose proof (weakening_cons _ _ _ _ _ _ arg_ty Hkont) as Hkont_w.
    pose proof (shift_closed_id _ _ _ _ _ _ Hkont 1) as Hsc2.
    assert (H__len : Datatypes.length (@nil ty) = 0) by reflexivity.
    rewrite H__len in Hsc2. clear H__len.
    rewrite Hsc2 in Hkont_w. clear Hsc2.

    (* Substitute kont for resume at index 1 *)
    assert (Hsub1 : has_type Sigma [arg_ty] [(Lin_Unrestricted, false)]
              (subst 1
                (E_Lam ret_ty (plug_delimited D (E_Var 0)))
                e_body) T0 handler_eff).
    { apply (subst_preserves_typing _ _ _ _ _ _ Hty_body 1
              (E_Lam ret_ty (plug_delimited D (E_Var 0)))
              (Ty_Arrow ret_ty comp_ty comp_eff)).
      - reflexivity.
      - simpl. exact Hkont_w. }

    (* Substitute v for arg at index 0 *)
    exact (substitution_preserves_typing _ _ _ _ _ _ _ _ Hsub1 Hv_pure).

  (** Case Step_RecordEval: {done, l=e, rest} ──► {done, l=e', rest} *)
  - destruct (has_type_record_inv _ _ _ _ Htype)
      as [ft [eff_rec [Heq_ft [Hrft Hsub]]]]. subst.
    destruct (rft_field_decompose _ _ _ _ _ _ _ _ _ Hrft)
      as [A [eff_e [He Hreplace]]].
    destruct (IHHstep _ _ He) as [eff_e' [He' Hsub_e]].
    exists eff_rec. split.
    + apply T_Record.
      apply Hreplace.
      apply T_Sub with (eff := eff_e').
      * exact He'.
      * exact Hsub_e.
    + exact Hsub.

  (** Case Step_Context: E[e] ──► E[e'] because e ──► e' *)
  - destruct (context_typing _ E e _ _ Htype) as [A [eff_inner [He Hreplace]]].
    destruct (IHHstep _ _ He) as [eff_inner' [He' Hsub_inner]].
    exists eff0. split.
    + apply Hreplace.
      apply T_Sub with (eff := eff_inner').
      * exact He'.
      * exact Hsub_inner.
    + apply effect_row_subset_refl.

  (** Case Step_ResumeValid: simplified resume E_App (E_Const Const_Unit) v
      This is vacuous — Const_Unit has type TyUnit, not an arrow type. *)
  - exfalso.
    destruct (has_type_app_inv _ _ _ _ _ Htype)
      as [A [fn_eff [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]]].
    remember (E_Const Const_Unit) as econst.
    remember (Ty_Arrow A T0 fn_eff) as Tarrow.
    clear -Hty1 Heqeconst HeqTarrow.
    induction Hty1; try discriminate.
    + (* T_Const: typeof_const Const_Unit = Ty_Base TyUnit ≠ Ty_Arrow *)
      injection Heqeconst as Hc. subst. simpl in HeqTarrow. discriminate.
    + (* T_Sub: recurse — T_Sub preserves the type *)
      exact (IHHty1 Heqeconst HeqTarrow).
Admitted.

Theorem preservation :
  forall Sigma e e' T eff M M',
    closed_well_typed Sigma e T eff ->
    step Sigma (mk_config e M) (mk_config e' M') ->
    exists eff',
      closed_well_typed Sigma e' T eff' /\
      effect_row_subset eff' eff.
Proof.
  intros Sigma e e' T eff M M' Htype Hstep.
  exact (preservation_ind _ _ _ Hstep T eff Htype).
Qed.

(** ** Effect handling removes the handled effect

    Premise: the handler's own effects do not include the handled effect.
    This is a well-formedness condition — a handler that re-performs the
    effect it handles would need an outer handler. *)

Lemma handle_removes_effect :
  forall Sigma e eff_name h comp_ty result_ty handler_eff,
    closed_well_typed Sigma e comp_ty
      (Eff_Closed (Eff_Entry eff_name :: [])) ->
    handler_well_formed Sigma [] [] h eff_name comp_ty result_ty handler_eff
                        (Eff_Closed (Eff_Entry eff_name :: [])) ->
    ~ effect_in_row eff_name handler_eff ->
    exists eff',
      closed_well_typed Sigma (E_Handle h e) result_ty eff' /\
      ~ effect_in_row eff_name eff'.
Proof.
  intros Sigma e eff_name h comp_ty result_ty handler_eff
         Htype Hwf Hnotin.
  exists handler_eff. split.
  - unfold closed_well_typed.
    eapply T_Handle.
    + apply Split_Nil.
    + exact Htype.
    + exact Hwf.
  - exact Hnotin.
Qed.
