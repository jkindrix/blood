(** * Blood Core Calculus — Type Inversion Lemmas

    Type inversion for closed well-typed expressions: given a specific
    expression form E_App, E_Let, E_Handle etc., extract the typing of
    subterms with effect subset witnesses.

    Also includes value typing inversion (values type with pure effect)
    and record field typing correspondence.

    Extracted from Preservation.v during modularization.
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

(** ** Value typing inversion (proved by mutual induction)

    Values type with pure effect. The record case requires mutual
    induction with record_fields_typed. *)

Lemma value_typing_inversion :
  forall Sigma Gamma Delta v T eff,
    has_type Sigma Gamma Delta v T eff ->
    is_value v = true ->
    has_type Sigma Gamma Delta v T Eff_Pure.
Proof.
  apply (has_type_mut_ind
    (fun Sigma Gamma Delta e T eff _ =>
       is_value e = true ->
       has_type Sigma Gamma Delta e T Eff_Pure)
    (fun Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff _ =>
       True)
    (fun Sigma Gamma Delta clauses eff_name eff_sig result_ty handler_eff _ =>
       True)
    (fun Sigma Gamma Delta fields field_types eff _ =>
       forallb (fun '(_, e) => is_value e) fields = true ->
       record_fields_typed Sigma Gamma Delta fields field_types Eff_Pure)).
  - (* T_Var *) intros ? ? ? ? ? Hlook Hval. simpl in Hval. discriminate.
  - (* T_Const *) intros. apply T_Const.
  - (* T_Lam *) intros Sigma Gamma Delta A B eff body Hbody IH Hval. apply T_Lam. exact Hbody.
  - (* T_App *) intros ? ? ? ? ? ? ? ? ? ? ? Hsplit Hty1 IH1 Hty2 IH2 Hval. simpl in Hval. discriminate.
  - (* T_Let *) intros ? ? ? ? ? ? ? ? ? ? Hsplit Hty1 IH1 Hty2 IH2 Hval. simpl in Hval. discriminate.
  - (* T_Annot *) intros ? ? ? ? ? ? Hty IH Hval. simpl in Hval. discriminate.
  - (* T_Record *)
    intros Sigma Gamma Delta fields field_types eff Hfields IH Hval.
    simpl in Hval. apply T_Record. exact (IH Hval).
  - (* T_Select *) intros ? ? ? ? ? ? ? ? Hty IH Hlook Hval. simpl in Hval. discriminate.
  - (* T_Perform *) intros ? ? ? ? ? ? ? ? ? ? Hlookeff Hlookop Hty IH Hval. simpl in Hval. discriminate.
  - (* T_Handle *) intros ? ? ? ? ? ? ? ? ? ? ? Hsplit Hty IH Hwf IHwf Hval. simpl in Hval. discriminate.
  - (* T_Sub *)
    intros Sigma Gamma Delta e T eff eff' Hty IH Hsub Hval.
    apply T_Sub with (eff := Eff_Pure).
    + exact (IH Hval).
    + simpl. trivial.
  - (* HWF *) intros. exact I.
  - (* OpClauses_Nil *) intros. exact I.
  - (* OpClauses_Cons *) intros. exact I.
  - (* RFT_Nil *) intros. apply RFT_Nil.
  - (* RFT_Cons *)
    intros Sigma Gamma Delta l e T rest_e rest_t eff1 eff2
           Hty IHe Hrest IHrest Hvals.
    simpl in Hvals.
    apply Bool.andb_true_iff in Hvals.
    destruct Hvals as [Hval1 Hval2].
    apply RFT_Cons with (eff1 := Eff_Pure) (eff2 := Eff_Pure).
    + exact (IHe Hval1).
    + exact (IHrest Hval2).
Qed.

Lemma record_fields_typed_pure :
  forall Sigma Gamma Delta fields field_types eff,
    record_fields_typed Sigma Gamma Delta fields field_types eff ->
    forallb (fun '(_, e) => is_value e) fields = true ->
    record_fields_typed Sigma Gamma Delta fields field_types Eff_Pure.
Proof.
  intros Sigma Gamma Delta fields field_types eff Htyped Hvals.
  induction Htyped.
  - apply RFT_Nil.
  - simpl in Hvals. apply Bool.andb_true_iff in Hvals. destruct Hvals as [Hval1 Hval2].
    apply RFT_Cons with (eff1 := Eff_Pure) (eff2 := Eff_Pure).
    + apply value_typing_inversion with (eff := eff1); assumption.
    + apply IHHtyped. assumption.
Qed.

(** ** value_to_expr always produces syntactic values *)

Axiom continuation_expr_is_value :
  forall e snap, is_value (value_to_expr (V_Continuation e snap)) = true.

Lemma value_to_expr_is_value :
  forall v, is_value (value_to_expr v) = true.
Proof.
  fix IH 1. destruct v as [c | t e | fields | e snap].
  - (* V_Const *) reflexivity.
  - (* V_Lam *) reflexivity.
  - (* V_Record *)
    simpl. induction fields as [| [l v'] fields' IHf].
    + reflexivity.
    + simpl. rewrite IH. simpl. exact IHf.
  - (* V_Continuation *) apply continuation_expr_is_value.
Qed.

(** ** Type inversion lemmas for closed terms

    These handle T_Sub layers: given a closed well-typed expression of a
    specific form, extract the typing of its subterms with an effect
    subset witness. *)

Lemma has_type_annot_inv :
  forall Sigma e T0 T eff,
    has_type Sigma [] [] (E_Annot e T0) T eff ->
    T = T0 /\
    exists eff_inner,
      has_type Sigma [] [] e T0 eff_inner /\
      effect_row_subset eff_inner eff.
Proof.
  intros Sigma e T0 T eff Hty.
  remember (E_Annot e T0) as eannot.
  induction Hty; try discriminate.
  - (* T_Annot *)
    injection Heqeannot as He HT. subst.
    split. reflexivity.
    exists eff. split. exact Hty. apply effect_row_subset_refl.
  - (* T_Sub *)
    destruct (IHHty Heqeannot) as [HeqT [eff_inner [Hinner Hsub_inner]]].
    subst. split. reflexivity.
    exists eff_inner. split. exact Hinner.
    apply effect_row_subset_trans with (e2 := eff); assumption.
Qed.

Lemma has_type_app_inv :
  forall Sigma e1 e2 T eff,
    has_type Sigma [] [] (E_App e1 e2) T eff ->
    exists A fn_eff eff1 eff2,
      has_type Sigma [] [] e1 (Ty_Arrow A T fn_eff) eff1 /\
      has_type Sigma [] [] e2 A eff2 /\
      effect_row_subset (effect_row_union fn_eff (effect_row_union eff1 eff2)) eff.
Proof.
  intros Sigma e1 e2 T eff Hty.
  remember (E_App e1 e2) as eapp.
  remember (@nil ty) as Gamma.
  remember (@nil (linearity * bool)) as Delta.
  generalize dependent HeqDelta.
  generalize dependent HeqGamma.
  generalize dependent e2.
  generalize dependent e1.
  induction Hty; intros; subst; try discriminate.
  - (* T_App *)
    injection Heqeapp as He1 He2. subst.
    apply lin_split_nil_inv in H as [HD1 HD2]. subst.
    exists A, fn_eff, eff1, eff2.
    split. exact Hty1. split. exact Hty2. apply effect_row_subset_refl.
  - (* T_Sub *)
    destruct (IHHty _ _ eq_refl eq_refl eq_refl)
      as [A [fn_eff [eff1 [eff2 [Hty1 [Hty2 Hsub_inner]]]]]].
    exists A, fn_eff, eff1, eff2.
    split. exact Hty1. split. exact Hty2.
    eapply effect_row_subset_trans; eassumption.
Qed.

Lemma has_type_lam_inv :
  forall Sigma T0 body T eff,
    has_type Sigma [] [] (E_Lam T0 body) T eff ->
    exists B fn_eff,
      T = Ty_Arrow T0 B fn_eff /\
      has_type Sigma [T0] [(Lin_Unrestricted, false)] body B fn_eff.
Proof.
  intros Sigma T0 body T eff Hty.
  remember (E_Lam T0 body) as elam.
  induction Hty; try discriminate.
  - (* T_Lam *)
    injection Heqelam as HT0 Hbody. subst.
    eexists. eexists. split. reflexivity. eassumption.
  - (* T_Sub *)
    exact (IHHty Heqelam).
Qed.

Lemma has_type_let_inv :
  forall Sigma e1 e2 T eff,
    has_type Sigma [] [] (E_Let e1 e2) T eff ->
    exists A eff1 eff2,
      has_type Sigma [] [] e1 A eff1 /\
      has_type Sigma [A] [(Lin_Unrestricted, false)] e2 T eff2 /\
      effect_row_subset (effect_row_union eff1 eff2) eff.
Proof.
  intros Sigma e1 e2 T eff Hty.
  remember (E_Let e1 e2) as elet.
  remember (@nil ty) as Gamma.
  remember (@nil (linearity * bool)) as Delta.
  generalize dependent HeqDelta.
  generalize dependent HeqGamma.
  generalize dependent e2.
  generalize dependent e1.
  induction Hty; intros; subst; try discriminate.
  - (* T_Let *)
    injection Heqelet as He1 He2. subst.
    apply lin_split_nil_inv in H as [HD1 HD2]. subst.
    exists A, eff1, eff2.
    split. exact Hty1. split. exact Hty2. apply effect_row_subset_refl.
  - (* T_Sub *)
    destruct (IHHty _ _ eq_refl eq_refl eq_refl)
      as [A [eff1 [eff2 [Hty1 [Hty2 Hsub_inner]]]]].
    exists A, eff1, eff2.
    split. exact Hty1. split. exact Hty2.
    eapply effect_row_subset_trans; eassumption.
Qed.

Lemma has_type_handle_inv :
  forall Sigma h e T eff,
    has_type Sigma [] [] (E_Handle h e) T eff ->
    exists eff_name comp_ty handler_eff comp_eff,
      has_type Sigma [] [] e comp_ty comp_eff /\
      handler_well_formed Sigma [] [] h eff_name comp_ty T handler_eff /\
      effect_row_subset handler_eff eff.
Proof.
  intros Sigma h e T eff Hty.
  remember (E_Handle h e) as ehandle.
  remember (@nil ty) as Gamma.
  remember (@nil (linearity * bool)) as Delta.
  generalize dependent HeqDelta.
  generalize dependent HeqGamma.
  generalize dependent e.
  generalize dependent h.
  induction Hty; intros; subst; try discriminate.
  - (* T_Handle *)
    injection Heqehandle as Hh He. subst.
    apply lin_split_nil_inv in H as [HD1 HD2]. subst.
    exists eff_name, comp_ty, handler_eff, comp_eff.
    split. exact Hty. split. exact H0. apply effect_row_subset_refl.
  - (* T_Sub *)
    destruct (IHHty _ _ eq_refl eq_refl eq_refl)
      as [en [ct [he [ce [Htye [Hwf Hsub_inner]]]]]].
    exists en, ct, he, ce.
    split. exact Htye. split. exact Hwf.
    eapply effect_row_subset_trans; eassumption.
Qed.

(** ** Record type inversion *)

Lemma has_type_record_inv :
  forall Sigma fields T eff,
    has_type Sigma [] [] (E_Record fields) T eff ->
    exists field_types eff_inner,
      T = Ty_Record field_types /\
      record_fields_typed Sigma [] [] fields field_types eff_inner /\
      effect_row_subset eff_inner eff.
Proof.
  intros Sigma fields T eff Hty.
  remember (E_Record fields) as erec.
  induction Hty; try discriminate.
  - (* T_Record *)
    injection Heqerec as Hfields. subst.
    exists field_types, eff. split. reflexivity. split. exact H.
    apply effect_row_subset_refl.
  - (* T_Sub *)
    destruct (IHHty Heqerec) as [ft [ei [Heq [Hrft Hsub_inner]]]].
    exists ft, ei. split. exact Heq. split. exact Hrft.
    eapply effect_row_subset_trans; eassumption.
Qed.

(** ** Record type inversion (general context) *)

Lemma has_type_record_inv_gen :
  forall Sigma Gamma Delta fields T eff,
    has_type Sigma Gamma Delta (E_Record fields) T eff ->
    exists field_types eff_inner,
      T = Ty_Record field_types /\
      record_fields_typed Sigma Gamma Delta fields field_types eff_inner /\
      effect_row_subset eff_inner eff.
Proof.
  intros Sigma Gamma Delta fields T eff Hty.
  remember (E_Record fields) as erec.
  induction Hty; try discriminate.
  - (* T_Record *)
    injection Heqerec as Hfields. subst.
    exists field_types, eff. split. reflexivity. split. exact H.
    apply effect_row_subset_refl.
  - (* T_Sub *)
    destruct (IHHty Heqerec) as [ft [ei [Heq [Hrft Hsub_inner]]]].
    exists ft, ei. split. exact Heq. split. exact Hrft.
    eapply effect_row_subset_trans; eassumption.
Qed.

(** ** Select type inversion *)

Lemma has_type_select_inv :
  forall Sigma e l T eff,
    has_type Sigma [] [] (E_Select e l) T eff ->
    exists field_types eff_inner,
      has_type Sigma [] [] e (Ty_Record field_types) eff_inner /\
      lookup_field field_types l = Some T /\
      effect_row_subset eff_inner eff.
Proof.
  intros Sigma e l T eff Hty.
  remember (E_Select e l) as esel.
  induction Hty; try discriminate.
  - (* T_Select *)
    injection Heqesel as He Hl. subst.
    exists fields, eff. split. exact Hty. split. exact H. apply effect_row_subset_refl.
  - (* T_Sub *)
    destruct (IHHty Heqesel) as [ft [ei [Htye [Hlook Hsub_inner]]]].
    exists ft, ei. split. exact Htye. split. exact Hlook.
    eapply effect_row_subset_trans; eassumption.
Qed.

(** ** Record field typing correspondence *)

Lemma record_fields_typed_find :
  forall Sigma Gamma Delta fields field_types eff l e T,
    record_fields_typed Sigma Gamma Delta fields field_types eff ->
    find_field fields l = Some e ->
    lookup_field field_types l = Some T ->
    has_type Sigma Gamma Delta e T eff.
Proof.
  intros Sigma Gamma Delta fields field_types eff l e T Htyped.
  induction Htyped; intros Hfind Hlook.
  - (* RFT_Nil *) simpl in Hfind. discriminate.
  - (* RFT_Cons *)
    simpl in Hfind. simpl in Hlook.
    destruct (label_eqb l0 l) eqn:Heql.
    + (* l0 = l: first match in both *)
      injection Hfind as He. injection Hlook as HT. subst.
      apply T_Sub with (eff := eff1).
      * exact H.
      * apply effect_row_subset_union_l.
    + (* l0 ≠ l: recurse into rest *)
      apply T_Sub with (eff := eff2).
      * apply IHHtyped; assumption.
      * apply effect_row_subset_union_r.
Qed.

(** ** Perform type inversion *)

Lemma has_type_perform_inv :
  forall Sigma eff_nm op_nm arg T eff,
    has_type Sigma [] [] (E_Perform eff_nm op_nm arg) T eff ->
    exists eff_sig arg_ty ret_ty eff_arg,
      lookup_effect Sigma eff_nm = Some eff_sig /\
      lookup_op eff_sig op_nm = Some (arg_ty, ret_ty) /\
      T = ret_ty /\
      has_type Sigma [] [] arg arg_ty eff_arg /\
      effect_row_subset
        (effect_row_union (Eff_Closed [Eff_Entry eff_nm]) eff_arg) eff.
Proof.
  intros Sigma eff_nm op_nm arg T eff Hty.
  remember (E_Perform eff_nm op_nm arg) as eperf.
  remember (@nil ty) as Gamma.
  remember (@nil (linearity * bool)) as Delta.
  induction Hty; try discriminate.
  - (* T_Perform *)
    injection Heqeperf as H1 H2 H3. subst.
    exists eff_sig, arg_ty, ret_ty, eff'.
    split. assumption. split. assumption. split. reflexivity.
    split. assumption. apply effect_row_subset_refl.
  - (* T_Sub *)
    destruct (IHHty Heqeperf HeqGamma HeqDelta)
      as [es [at' [rt [ef [H1 [H2 [H3 [H4 H5]]]]]]]].
    exists es, at', rt, ef.
    split. assumption. split. assumption. split. assumption.
    split. assumption.
    eapply effect_row_subset_trans; eassumption.
Qed.

(** ** Generalized inversion lemmas (arbitrary Gamma/Delta)

    These are needed for delimited_context_typing_gen, which works
    in non-empty contexts (required for the HandleOp preservation case). *)

Lemma has_type_app_inv_gen :
  forall Sigma Gamma Delta e1 e2 T eff,
    has_type Sigma Gamma Delta (E_App e1 e2) T eff ->
    exists A Delta1 Delta2 fn_eff eff1 eff2,
      lin_split Delta Delta1 Delta2 /\
      has_type Sigma Gamma Delta1 e1 (Ty_Arrow A T fn_eff) eff1 /\
      has_type Sigma Gamma Delta2 e2 A eff2 /\
      effect_row_subset (effect_row_union fn_eff (effect_row_union eff1 eff2)) eff.
Proof.
  intros Sigma Gamma Delta e1 e2 T eff Hty.
  remember (E_App e1 e2) as eapp.
  induction Hty; try discriminate.
  - injection Heqeapp as He1 He2. subst.
    exists A, Delta1, Delta2, fn_eff, eff1, eff2.
    split. exact H. split. exact Hty1. split. exact Hty2.
    apply effect_row_subset_refl.
  - destruct (IHHty Heqeapp)
      as [A0 [D1 [D2 [fe [e1' [e2' [Hs [H1 [H2 Hsub']]]]]]]]].
    exists A0, D1, D2, fe, e1', e2'.
    split. exact Hs. split. exact H1. split. exact H2.
    eapply effect_row_subset_trans; eassumption.
Qed.

Lemma has_type_let_inv_gen :
  forall Sigma Gamma Delta e1 e2 T eff,
    has_type Sigma Gamma Delta (E_Let e1 e2) T eff ->
    exists A Delta1 Delta2 eff1 eff2,
      lin_split Delta Delta1 Delta2 /\
      has_type Sigma Gamma Delta1 e1 A eff1 /\
      has_type Sigma (A :: Gamma) ((Lin_Unrestricted, false) :: Delta2) e2 T eff2 /\
      effect_row_subset (effect_row_union eff1 eff2) eff.
Proof.
  intros Sigma Gamma Delta e1 e2 T eff Hty.
  remember (E_Let e1 e2) as elet.
  induction Hty; try discriminate.
  - injection Heqelet as He1 He2. subst.
    exists A, Delta1, Delta2, eff1, eff2.
    split. exact H. split. exact Hty1. split. exact Hty2.
    apply effect_row_subset_refl.
  - destruct (IHHty Heqelet)
      as [A0 [D1 [D2 [e1' [e2' [Hs [H1 [H2 Hsub']]]]]]]].
    exists A0, D1, D2, e1', e2'.
    split. exact Hs. split. exact H1. split. exact H2.
    eapply effect_row_subset_trans; eassumption.
Qed.

Lemma has_type_select_inv_gen :
  forall Sigma Gamma Delta e l T eff,
    has_type Sigma Gamma Delta (E_Select e l) T eff ->
    exists field_types eff_inner,
      has_type Sigma Gamma Delta e (Ty_Record field_types) eff_inner /\
      lookup_field field_types l = Some T /\
      effect_row_subset eff_inner eff.
Proof.
  intros Sigma Gamma Delta e l T eff Hty.
  remember (E_Select e l) as esel.
  induction Hty; try discriminate.
  - injection Heqesel as He Hl. subst.
    exists fields, eff. split. exact Hty. split. exact H.
    apply effect_row_subset_refl.
  - destruct (IHHty Heqesel) as [ft [ei [H1 [H2 H3]]]].
    exists ft, ei. split. exact H1. split. exact H2.
    eapply effect_row_subset_trans; eassumption.
Qed.

Lemma has_type_annot_inv_gen :
  forall Sigma Gamma Delta e T0 T eff,
    has_type Sigma Gamma Delta (E_Annot e T0) T eff ->
    T = T0 /\
    exists eff_inner,
      has_type Sigma Gamma Delta e T0 eff_inner /\
      effect_row_subset eff_inner eff.
Proof.
  intros Sigma Gamma Delta e T0 T eff Hty.
  remember (E_Annot e T0) as eannot.
  induction Hty; try discriminate.
  - injection Heqeannot as He HT. subst.
    split. reflexivity. exists eff. split. exact Hty.
    apply effect_row_subset_refl.
  - destruct (IHHty Heqeannot) as [HeqT [ei [Hi Hsub']]]. subst.
    split. reflexivity. exists ei. split. exact Hi.
    eapply effect_row_subset_trans; eassumption.
Qed.

Lemma has_type_perform_inv_gen :
  forall Sigma Gamma Delta eff_nm op_nm arg T eff,
    has_type Sigma Gamma Delta (E_Perform eff_nm op_nm arg) T eff ->
    exists eff_sig arg_ty ret_ty eff_arg,
      lookup_effect Sigma eff_nm = Some eff_sig /\
      lookup_op eff_sig op_nm = Some (arg_ty, ret_ty) /\
      T = ret_ty /\
      has_type Sigma Gamma Delta arg arg_ty eff_arg /\
      effect_row_subset
        (effect_row_union (Eff_Closed [Eff_Entry eff_nm]) eff_arg) eff.
Proof.
  intros Sigma Gamma Delta eff_nm op_nm arg T eff Hty.
  remember (E_Perform eff_nm op_nm arg) as eperf.
  induction Hty; try discriminate.
  - injection Heqeperf as H1 H2 H3. subst.
    exists eff_sig, arg_ty, ret_ty, eff'.
    split. assumption. split. assumption. split. reflexivity.
    split. assumption. apply effect_row_subset_refl.
  - destruct (IHHty Heqeperf)
      as [es [at' [rt [ef [H1 [H2 [H3 [H4 H5]]]]]]]].
    exists es, at', rt, ef.
    split. assumption. split. assumption. split. assumption.
    split. assumption.
    eapply effect_row_subset_trans; eassumption.
Qed.

(** ** Record field decomposition and replacement

    Given record_fields_typed for (done ++ (l, e) :: rest), extract
    the typing of field e and provide a replacement function. *)

Lemma rft_field_decompose :
  forall Sigma Gamma Delta done l e rest ft eff,
    record_fields_typed Sigma Gamma Delta (done ++ (l, e) :: rest) ft eff ->
    exists A eff_e,
      has_type Sigma Gamma Delta e A eff_e /\
      forall e',
        has_type Sigma Gamma Delta e' A eff_e ->
        record_fields_typed Sigma Gamma Delta (done ++ (l, e') :: rest) ft eff.
Proof.
  induction done as [| [l0 e0] done' IH]; intros l e rest ft eff Hrft.
  - (* Base: done = [], field is at head *)
    simpl in *.
    inversion Hrft as [| ? ? ? ? ? T0 ? ? eff1_ eff2_ Hty_e Hrft_rest]; subst.
    exists T0, eff1_. split.
    + exact Hty_e.
    + intros e' He'. apply RFT_Cons; [exact He' | exact Hrft_rest].
  - (* Step: done = (l0, e0) :: done' *)
    simpl in *.
    inversion Hrft as [| ? ? ? ? ? T0 ? ? eff1_ eff2_ Hty_e0 Hrft_rest]; subst.
    destruct (IH _ _ _ _ _ Hrft_rest) as [A [eff_e [He Hreplace]]].
    exists A, eff_e. split.
    + exact He.
    + intros e' He'. apply RFT_Cons; [exact Hty_e0 | exact (Hreplace e' He')].
Qed.

(** Field effect is a subset of the overall record effect *)

Lemma rft_field_effect_incl :
  forall Sigma Gamma Delta done l e rest ft eff,
    record_fields_typed Sigma Gamma Delta (done ++ (l, e) :: rest) ft eff ->
    exists A eff_e,
      has_type Sigma Gamma Delta e A eff_e /\
      effect_row_subset eff_e eff.
Proof.
  induction done as [| [l0 e0] done' IH]; intros l e rest ft eff Hrft.
  - simpl in *.
    inversion Hrft as [| ? ? ? ? ? T0 ? ? eff1_ eff2_ Hty_e Hrft_rest]; subst.
    exists T0, eff1_. split.
    + exact Hty_e.
    + apply effect_row_subset_union_l.
  - simpl in *.
    inversion Hrft as [| ? ? ? ? ? T0 ? ? eff1_ eff2_ Hty_e0 Hrft_rest]; subst.
    destruct (IH _ _ _ _ _ Hrft_rest) as [A [eff_e [He Hsub_tail]]].
    exists A, eff_e. split.
    + exact He.
    + eapply effect_row_subset_trans.
      * exact Hsub_tail.
      * apply effect_row_subset_union_r.
Qed.
