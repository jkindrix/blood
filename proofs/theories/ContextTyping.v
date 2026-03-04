(** * Blood Core Calculus — Context Typing

    Typing decomposition for evaluation contexts E[e] and delimited
    contexts D[e]: if the plugged expression is well-typed, extract the
    type of the hole and a replacement function.

    Also includes perform_requires_effect, plug_delimited_perform_effect,
    effect_in_row_not_pure, and op_clause_typing_lookup.

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
From Blood Require Import Semantics.
From Blood Require Import EffectAlgebra.
From Blood Require Import Inversion.
From Blood Require Import Substitution.

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
  induction E; intros e0 T eff Hty; simpl in *.

  (** EC_Hole: plug = identity *)
  - exists T, eff. split.
    + exact Hty.
    + intros e' He'. exact He'.

  (** EC_AppFun E' e2: plug = E_App (plug E' e0) e2 *)
  - destruct (has_type_app_inv _ _ _ _ _ Hty)
      as [A [fn_eff [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]]].
    destruct (IHE _ _ _ Hty1) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union fn_eff (effect_row_union eff1 eff2)).
      * apply T_App with (Delta1 := []) (Delta2 := [])
                          (A := A) (fn_eff := fn_eff)
                          (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact (Hreplace e' He').
        -- exact Hty2.
      * exact Hsub.

  (** EC_AppArg v E': plug = E_App (value_to_expr v) (plug E' e0) *)
  - destruct (has_type_app_inv _ _ _ _ _ Hty)
      as [A [fn_eff [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]]].
    destruct (IHE _ _ _ Hty2) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union fn_eff (effect_row_union eff1 eff2)).
      * apply T_App with (Delta1 := []) (Delta2 := [])
                          (A := A) (fn_eff := fn_eff)
                          (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact Hty1.
        -- exact (Hreplace e' He').
      * exact Hsub.

  (** EC_Let E' e2: plug = E_Let (plug E' e0) e2 *)
  - destruct (has_type_let_inv _ _ _ _ _ Hty)
      as [A [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]].
    destruct (IHE _ _ _ Hty1) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * apply T_Let with (Delta1 := []) (Delta2 := [])
                          (A := A) (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact (Hreplace e' He').
        -- exact Hty2.
      * exact Hsub.

  (** EC_Select E' l: plug = E_Select (plug E' e0) l *)
  - destruct (has_type_select_inv _ _ _ _ _ Hty)
      as [ft [eff_inner' [Hty_rec [Hlook Hsub]]]].
    destruct (IHE _ _ _ Hty_rec) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * apply T_Select with (fields := ft).
        -- exact (Hreplace e' He').
        -- exact Hlook.
      * exact Hsub.

  (** EC_Annot E' T0: plug = E_Annot (plug E' e0) T0 *)
  - destruct (has_type_annot_inv _ _ _ _ _ Hty)
      as [HeqT [eff_inner' [Hinner Hsub]]]. subst.
    destruct (IHE _ _ _ Hinner) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * apply T_Annot. exact (Hreplace e' He').
      * exact Hsub.

  (** EC_PerformArg eff_nm op_nm E': plug = E_Perform eff_nm op_nm (plug E' e0) *)
  - remember (E_Perform e o (plug_eval E e0)) as eperf.
    (* Inline perform inversion *)
    assert (Hperf_inv : exists eff_sig arg_ty ret_ty eff',
      lookup_effect Sigma e = Some eff_sig /\
      lookup_op eff_sig o = Some (arg_ty, ret_ty) /\
      T = ret_ty /\
      has_type Sigma [] [] (plug_eval E e0) arg_ty eff' /\
      effect_row_subset
        (effect_row_union (Eff_Closed [Eff_Entry e]) eff') eff).
    { clear IHE.
      remember (@nil ty) as Gamma.
      remember (@nil (linearity * bool)) as Delta.
      induction Hty; try discriminate.
      - (* T_Perform *)
        injection Heqeperf as Heff Hop Harg. subst.
        exists eff_sig, arg_ty, ret_ty, eff'.
        split. assumption. split. assumption. split. reflexivity.
        split. assumption. apply effect_row_subset_refl.
      - (* T_Sub *)
        destruct (IHHty Heqeperf HeqGamma HeqDelta)
          as [es [at' [rt [ef [H1 [H2 [H3 [H4 H5]]]]]]]].
        exists es, at', rt, ef.
        split. assumption. split. assumption. split. assumption.
        split. assumption.
        eapply effect_row_subset_trans; eassumption. }
    destruct Hperf_inv as [eff_sig [arg_ty [ret_ty [eff' [Hlook_eff [Hlook_op [HeqT [Hty_arg Hsub]]]]]]]].
    subst T.
    destruct (IHE _ _ _ Hty_arg) as [A' [eff_inner [He1 Hreplace]]].
    exists A', eff_inner. split.
    + exact He1.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union (Eff_Closed [Eff_Entry e]) eff').
      * apply T_Perform with (eff_sig := eff_sig) (arg_ty := arg_ty).
        -- exact Hlook_eff.
        -- exact Hlook_op.
        -- exact (Hreplace e' He').
      * exact Hsub.

  (** EC_Handle h E': plug = E_Handle h (plug E' e0) *)
  - destruct (has_type_handle_inv _ _ _ _ _ Hty)
      as [en [ct [he [ce [Htye [Hwf [Hpass Hsub]]]]]]].
    destruct (IHE _ _ _ Htye) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := he).
      * apply T_Handle with (Delta1 := []) (Delta2 := [])
                             (eff_name := en) (comp_ty := ct)
                             (handler_eff := he) (comp_eff := ce).
        -- apply Split_Nil.
        -- exact (Hreplace e' He').
        -- exact Hwf.
        -- exact Hpass.
      * exact Hsub.

  (** EC_ExtendVal l E' e2: plug = E_Extend l (plug_eval E' e0) e2 *)
  - destruct (has_type_extend_inv _ _ _ _ _ _ Hty)
      as [T1 [fields [eff1 [eff2 [Heq [Hty1 [Hty2 Hsub]]]]]]]. subst.
    destruct (IHE _ _ _ Hty1) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * apply T_Extend with (Delta1 := []) (Delta2 := [])
                              (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact (Hreplace e' He').
        -- exact Hty2.
      * exact Hsub.

  (** EC_ExtendRec l v E': plug = E_Extend l (value_to_expr v) (plug_eval E' e0) *)
  - destruct (has_type_extend_inv _ _ _ _ _ _ Hty)
      as [T1 [fields [eff1 [eff2 [Heq [Hty1 [Hty2 Hsub]]]]]]]. subst.
    destruct (IHE _ _ _ Hty2) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * apply T_Extend with (Delta1 := []) (Delta2 := [])
                              (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact Hty1.
        -- exact (Hreplace e' He').
      * exact Hsub.

  (** EC_Resume E': plug = E_Resume (plug_eval E' e0) *)
  - destruct (has_type_resume_inv _ _ _ _ Hty)
      as [eff_inner' [Hinner Hsub]].
    destruct (IHE _ _ _ Hinner) as [A' [eff_inner [He0 Hreplace]]].
    exists A', eff_inner. split.
    + exact He0.
    + intros e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * apply T_Resume. exact (Hreplace e' He').
      * exact Hsub.
Qed.

(** ** Perform at top level requires effect in row *)

Lemma perform_requires_effect :
  forall Sigma eff_nm op arg T eff,
    has_type Sigma [] [] (E_Perform eff_nm op arg) T eff ->
    effect_in_row eff_nm eff.
Proof.
  intros Sigma eff_nm op arg T eff Htype.
  remember (E_Perform eff_nm op arg) as eperf.
  remember (@nil ty) as Gamma.
  remember (@nil (linearity * bool)) as Delta.
  induction Htype; try discriminate.
  - (* T_Perform *)
    injection Heqeperf as H1 H2 H3. subst.
    simpl.
    destruct eff'; simpl.
    + left. reflexivity.
    + unfold effect_entries_union.
      destruct (existsb (fun e0 => match e0 with Eff_Entry n' => effect_name_eqb eff_nm n' end) l) eqn:Hex.
      * apply existsb_exists in Hex.
        destruct Hex as [[n'] [Hin Heqb]].
        unfold effect_name_eqb in Heqb.
        apply String.eqb_eq in Heqb. subst n'.
        simpl. exact Hin.
      * simpl. left. reflexivity.
    + unfold effect_entries_union.
      destruct (existsb (fun e0 => match e0 with Eff_Entry n' => effect_name_eqb eff_nm n' end) l) eqn:Hex.
      * apply existsb_exists in Hex.
        destruct Hex as [[n'] [Hin Heqb]].
        unfold effect_name_eqb in Heqb.
        apply String.eqb_eq in Heqb. subst n'.
        simpl. exact Hin.
      * simpl. left. reflexivity.
  - (* T_Sub *)
    subst.
    assert (Hin : effect_in_row eff_nm eff).
    { apply IHHtype; auto. }
    destruct eff, eff'; simpl in *;
      try contradiction;
      try (subst; inversion Hin; fail);
      try (apply H; exact Hin);
      auto.
Qed.

(** ** effect_in_row is incompatible with sub-pure effects *)

Lemma effect_in_row_not_pure :
  forall eff_nm eff,
    effect_in_row eff_nm eff ->
    effect_row_subset eff Eff_Pure ->
    False.
Proof.
  intros eff_nm eff Hin Hsub.
  destruct eff; simpl in *.
  - exact Hin.
  - subst. inversion Hin.
  - exact Hsub.
Qed.

(** ** Op clause typing lookup

    If a clause is in a well-typed clause list, extract its typing. *)

Lemma op_clause_typing_lookup :
  forall Sigma Gamma Delta clauses eff_name sig rrt re result_ty handler_eff
         eff_nm op_nm e_body,
    op_clauses_well_formed Sigma Gamma Delta clauses eff_name sig rrt re result_ty handler_eff ->
    In (OpClause eff_nm op_nm e_body) clauses ->
    eff_nm = eff_name /\
    exists arg_ty ret_ty,
      lookup_op sig op_nm = Some (arg_ty, ret_ty) /\
      has_type Sigma
               (arg_ty :: Ty_Arrow ret_ty rrt re :: Gamma)
               ((Lin_Unrestricted, false) :: (Lin_Unrestricted, false) :: Delta)
               e_body result_ty handler_eff.
Proof.
  intros Sigma Gamma Delta clauses eff_name sig rrt re result_ty handler_eff
         eff_nm op_nm e_body Hwf Hin.
  induction Hwf.
  - (* OpClauses_Nil *) destruct Hin.
  - (* OpClauses_Cons *)
    destruct Hin as [Heq | Hin_rest].
    + (* Head match *)
      injection Heq as H1 H2 H3. subst.
      split. reflexivity.
      exists arg_ty, ret_ty.
      split; assumption.
    + (* In rest *)
      exact (IHHwf Hin_rest).
Qed.

(** ** Delimited context typing

    Analogous to context_typing for eval_contexts, but for
    delimited contexts (which don't cross handler boundaries). *)

Lemma delimited_context_typing :
  forall Sigma D e T eff,
    has_type Sigma [] [] (plug_delimited D e) T eff ->
    exists A eff_inner,
      has_type Sigma [] [] e A eff_inner /\
      forall e',
        has_type Sigma [] [] e' A eff_inner ->
        has_type Sigma [] [] (plug_delimited D e') T eff.
Proof.
  induction D as [
    | D' IHD e2_
    | v_ D' IHD
    | D' IHD e2_
    | D' IHD l_
    | D' IHD T_
    | en_ opn_ D' IHD
    | done_ l_ D' IHD rest_
    | h_ D' IHD
    | l_ D' IHD e2_
    | l_ v_ D' IHD
    | D' IHD
  ]; intros e T eff Hty; simpl in *.

  (** DC_Hole *)
  - exists T, eff. split.
    + exact Hty.
    + intros e' He'. exact He'.

  (** DC_AppFun D' e2_ *)
  - destruct (has_type_app_inv _ _ _ _ _ Hty)
      as [A [fn_eff [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]]].
    destruct (IHD _ _ _ Hty1) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union fn_eff (effect_row_union eff1 eff2)).
      * apply T_App with (Delta1 := []) (Delta2 := [])
                          (A := A) (fn_eff := fn_eff)
                          (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact (Hreplace e' He').
        -- exact Hty2.
      * exact Hsub.

  (** DC_AppArg v_ D' *)
  - destruct (has_type_app_inv _ _ _ _ _ Hty)
      as [A [fn_eff [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]]].
    destruct (IHD _ _ _ Hty2) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union fn_eff (effect_row_union eff1 eff2)).
      * apply T_App with (Delta1 := []) (Delta2 := [])
                          (A := A) (fn_eff := fn_eff)
                          (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact Hty1.
        -- exact (Hreplace e' He').
      * exact Hsub.

  (** DC_Let D' e2_ *)
  - destruct (has_type_let_inv _ _ _ _ _ Hty)
      as [A [eff1 [eff2 [Hty1 [Hty2 Hsub]]]]].
    destruct (IHD _ _ _ Hty1) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * apply T_Let with (Delta1 := []) (Delta2 := [])
                          (A := A) (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact (Hreplace e' He').
        -- exact Hty2.
      * exact Hsub.

  (** DC_Select D' l_ *)
  - destruct (has_type_select_inv _ _ _ _ _ Hty)
      as [ft [eff_inner' [Hty_rec [Hlook Hsub]]]].
    destruct (IHD _ _ _ Hty_rec) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * apply T_Select with (fields := ft).
        -- exact (Hreplace e' He').
        -- exact Hlook.
      * exact Hsub.

  (** DC_Annot D' T_ *)
  - destruct (has_type_annot_inv _ _ _ _ _ Hty)
      as [HeqT [eff_inner' [Hinner Hsub]]]. subst.
    destruct (IHD _ _ _ Hinner) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * apply T_Annot. exact (Hreplace e' He').
      * exact Hsub.

  (** DC_PerformArg en_ opn_ D' *)
  - remember (E_Perform en_ opn_ (plug_delimited D' e)) as eperf.
    (* Inline perform inversion *)
    assert (Hperf_inv : exists eff_sig arg_ty ret_ty eff',
      lookup_effect Sigma en_ = Some eff_sig /\
      lookup_op eff_sig opn_ = Some (arg_ty, ret_ty) /\
      T = ret_ty /\
      has_type Sigma [] [] (plug_delimited D' e) arg_ty eff' /\
      effect_row_subset
        (effect_row_union (Eff_Closed [Eff_Entry en_]) eff') eff).
    { clear IHD.
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
        eapply effect_row_subset_trans; eassumption. }
    destruct Hperf_inv as [eff_sig [arg_ty [ret_ty [eff' [Hlook_eff [Hlook_op [HeqT [Hty_arg Hsub]]]]]]]].
    subst T.
    destruct (IHD _ _ _ Hty_arg) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union (Eff_Closed [Eff_Entry en_]) eff').
      * apply T_Perform with (eff_sig := eff_sig) (arg_ty := arg_ty).
        -- exact Hlook_eff.
        -- exact Hlook_op.
        -- exact (Hreplace e' He').
      * exact Hsub.

  (** DC_RecordField done_ l_ D' rest_ *)
  - destruct (has_type_record_inv _ _ _ _ Hty)
      as [ft [eff_rec [Heq_ft [Hrft Hsub]]]]. subst.
    destruct (rft_field_decompose _ _ _ _ _ _ _ _ _ Hrft)
      as [A [eff_field [Hfield Hreplace_rft]]].
    destruct (IHD _ _ _ Hfield) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := eff_rec).
      * apply T_Record.
        apply Hreplace_rft. exact (Hreplace e' He').
      * exact Hsub.

  (** DC_HandleOther h_ D' *)
  - destruct (has_type_handle_inv _ _ _ _ _ Hty)
      as [en [ct [he [ce [Htye [Hwf [Hpass Hsub]]]]]]].
    destruct (IHD _ _ _ Htye) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := he).
      * apply T_Handle with (Delta1 := []) (Delta2 := [])
                             (eff_name := en) (comp_ty := ct)
                             (handler_eff := he) (comp_eff := ce).
        -- apply Split_Nil.
        -- exact (Hreplace e' He').
        -- exact Hwf.
        -- exact Hpass.
      * exact Hsub.

  (** DC_ExtendVal l_ D' e2_ *)
  - destruct (has_type_extend_inv _ _ _ _ _ _ Hty)
      as [T1 [fields [eff1 [eff2 [Heq [Hty1 [Hty2 Hsub]]]]]]]. subst.
    destruct (IHD _ _ _ Hty1) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * apply T_Extend with (Delta1 := []) (Delta2 := [])
                              (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact (Hreplace e' He').
        -- exact Hty2.
      * exact Hsub.

  (** DC_ExtendRec l_ v_ D' *)
  - destruct (has_type_extend_inv _ _ _ _ _ _ Hty)
      as [T1 [fields [eff1 [eff2 [Heq [Hty1 [Hty2 Hsub]]]]]]]. subst.
    destruct (IHD _ _ _ Hty2) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * apply T_Extend with (Delta1 := []) (Delta2 := [])
                              (eff1 := eff1) (eff2 := eff2).
        -- apply Split_Nil.
        -- exact Hty1.
        -- exact (Hreplace e' He').
      * exact Hsub.

  (** DC_Resume D' *)
  - destruct (has_type_resume_inv _ _ _ _ Hty)
      as [eff_inner' [Hinner Hsub]].
    destruct (IHD _ _ _ Hinner) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * apply T_Resume. exact (Hreplace e' He').
      * exact Hsub.
Qed.

(** ** Generalized delimited context typing (arbitrary Gamma/Delta)

    Required for the HandleOp preservation proof, where the continuation
    body lives in a non-empty context [ret_ty]. *)

Lemma delimited_context_typing_gen :
  forall Sigma Gamma Delta D e T eff,
    has_type Sigma Gamma Delta (plug_delimited D e) T eff ->
    exists A eff_inner,
      has_type Sigma Gamma Delta e A eff_inner /\
      forall Delta' e',
        has_type Sigma Gamma Delta' e' A eff_inner ->
        has_type Sigma Gamma Delta (plug_delimited D e') T eff.
Proof.
  induction D as [
    | D' IHD e2_
    | v_ D' IHD
    | D' IHD e2_
    | D' IHD l_
    | D' IHD T_
    | en_ opn_ D' IHD
    | done_ l_ D' IHD rest_
    | h_ D' IHD
    | l_ D' IHD e2_
    | l_ v_ D' IHD
    | D' IHD
  ]; intros e T eff Hty; simpl in *.

  (** DC_Hole *)
  - exists T, eff. split.
    + exact Hty.
    + intros Delta' e' He'.
      exact (has_type_lin_irrelevant _ _ _ _ _ _ He' Delta).

  (** DC_AppFun D' e2_ *)
  - destruct (has_type_app_inv_gen _ _ _ _ _ _ _ Hty)
      as [A [D1 [D2 [fn_eff [eff1 [eff2 [Hsplit [Hty1 [Hty2 Hsub]]]]]]]]].
    destruct (IHD _ _ _ (has_type_lin_irrelevant _ _ _ _ _ _ Hty1 Delta))
      as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := effect_row_union fn_eff (effect_row_union eff1 eff2)).
      * eapply T_App.
        -- exact Hsplit.
        -- exact (has_type_lin_irrelevant _ _ _ _ _ _ (Hreplace _ e' He') D1).
        -- exact Hty2.
      * exact Hsub.

  (** DC_AppArg v_ D' *)
  - destruct (has_type_app_inv_gen _ _ _ _ _ _ _ Hty)
      as [A [D1 [D2 [fn_eff [eff1 [eff2 [Hsplit [Hty1 [Hty2 Hsub]]]]]]]]].
    destruct (IHD _ _ _ (has_type_lin_irrelevant _ _ _ _ _ _ Hty2 Delta))
      as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := effect_row_union fn_eff (effect_row_union eff1 eff2)).
      * eapply T_App.
        -- exact Hsplit.
        -- exact Hty1.
        -- exact (has_type_lin_irrelevant _ _ _ _ _ _ (Hreplace _ e' He') D2).
      * exact Hsub.

  (** DC_Let D' e2_ *)
  - destruct (has_type_let_inv_gen _ _ _ _ _ _ _ Hty)
      as [A [D1 [D2 [eff1 [eff2 [Hsplit [Hty1 [Hty2 Hsub]]]]]]]].
    destruct (IHD _ _ _ (has_type_lin_irrelevant _ _ _ _ _ _ Hty1 Delta))
      as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * eapply T_Let.
        -- exact Hsplit.
        -- exact (has_type_lin_irrelevant _ _ _ _ _ _ (Hreplace _ e' He') D1).
        -- exact Hty2.
      * exact Hsub.

  (** DC_Select D' l_ *)
  - destruct (has_type_select_inv_gen _ _ _ _ _ _ _ Hty)
      as [ft [eff_inner' [Hty_rec [Hlook Hsub]]]].
    destruct (IHD _ _ _ Hty_rec) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * eapply T_Select; [exact (Hreplace _ e' He') | exact Hlook].
      * exact Hsub.

  (** DC_Annot D' T_ *)
  - destruct (has_type_annot_inv_gen _ _ _ _ _ _ _ Hty)
      as [HeqT [eff_inner' [Hinner Hsub]]]. subst.
    destruct (IHD _ _ _ Hinner) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * apply T_Annot. exact (Hreplace _ e' He').
      * exact Hsub.

  (** DC_PerformArg en_ opn_ D' *)
  - destruct (has_type_perform_inv_gen _ _ _ _ _ _ _ _ Hty)
      as [eff_sig [arg_ty [ret_ty [eff' [Hlook_eff [Hlook_op [HeqT [Hty_arg Hsub]]]]]]]].
    subst T.
    destruct (IHD _ _ _ Hty_arg) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := effect_row_union (Eff_Closed [Eff_Entry en_]) eff').
      * eapply T_Perform; [exact Hlook_eff | exact Hlook_op | exact (Hreplace _ e' He')].
      * exact Hsub.

  (** DC_RecordField done_ l_ D' rest_ *)
  - destruct (has_type_record_inv_gen _ _ _ _ _ _ Hty)
      as [ft [eff_rec [Heq_ft [Hrft Hsub]]]]. subst.
    destruct (rft_field_decompose _ _ _ _ _ _ _ _ _ Hrft)
      as [A [eff_field [Hfield Hreplace_rft]]].
    destruct (IHD _ _ _ Hfield) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := eff_rec).
      * apply T_Record.
        apply Hreplace_rft. exact (Hreplace _ e' He').
      * exact Hsub.

  (** DC_HandleOther h_ D' *)
  - destruct (has_type_handle_inv_gen _ _ _ _ _ _ _ Hty)
      as [en [ct [D1 [D2 [he [ce [Hsplit [Htye [Hwf [Hpass Hsub]]]]]]]]]].
    destruct (IHD _ _ _ (has_type_lin_irrelevant _ _ _ _ _ _ Htye Delta))
      as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := he).
      * eapply T_Handle.
        -- exact Hsplit.
        -- exact (has_type_lin_irrelevant _ _ _ _ _ _ (Hreplace _ e' He') D1).
        -- exact Hwf.
        -- exact Hpass.
      * exact Hsub.

  (** DC_ExtendVal l_ D' e2_ *)
  - destruct (has_type_extend_inv_gen _ _ _ _ _ _ _ _ Hty)
      as [T1 [fields [D1 [D2 [eff1 [eff2 [Heq [Hsplit [Hty1 [Hty2 Hsub]]]]]]]]]]. subst.
    destruct (IHD _ _ _ (has_type_lin_irrelevant _ _ _ _ _ _ Hty1 Delta))
      as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * eapply T_Extend.
        -- exact Hsplit.
        -- exact (has_type_lin_irrelevant _ _ _ _ _ _ (Hreplace _ e' He') D1).
        -- exact Hty2.
      * exact Hsub.

  (** DC_ExtendRec l_ v_ D' *)
  - destruct (has_type_extend_inv_gen _ _ _ _ _ _ _ _ Hty)
      as [T1 [fields [D1 [D2 [eff1 [eff2 [Heq [Hsplit [Hty1 [Hty2 Hsub]]]]]]]]]]. subst.
    destruct (IHD _ _ _ (has_type_lin_irrelevant _ _ _ _ _ _ Hty2 Delta))
      as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := effect_row_union eff1 eff2).
      * eapply T_Extend.
        -- exact Hsplit.
        -- exact Hty1.
        -- exact (has_type_lin_irrelevant _ _ _ _ _ _ (Hreplace _ e' He') D2).
      * exact Hsub.

  (** DC_Resume D' *)
  - destruct (has_type_resume_inv_gen _ _ _ _ _ _ Hty)
      as [eff_inner' [Hinner Hsub]].
    destruct (IHD _ _ _ Hinner) as [A' [eff_inner [He Hreplace]]].
    exists A', eff_inner. split.
    + exact He.
    + intros Delta' e' He'. simpl.
      apply T_Sub with (eff := eff_inner').
      * apply T_Resume. exact (Hreplace _ e' He').
      * exact Hsub.
Qed.

(** ** plug_delimited preserves effect containment

    If D[perform eff_nm.op(v)] is well-typed with effect eff,
    and no handler in D handles eff_nm, then eff_nm is in eff. *)

Lemma plug_delimited_perform_effect :
  forall Sigma D eff_nm op v T eff,
    has_type Sigma [] []
      (plug_delimited D (E_Perform eff_nm op (value_to_expr v))) T eff ->
    dc_no_match D eff_nm ->
    effect_in_row eff_nm eff.
Proof.
  intros Sigma D.
  induction D as [
    | D' IHD e2_
    | v_ D' IHD
    | D' IHD e2_
    | D' IHD l_
    | D' IHD T_
    | en_ opn_ D' IHD
    | done_ l_ D' IHD rest_
    | h_ D' IHD
    | l_ D' IHD e2_
    | l_ v_ D' IHD
    | D' IHD
  ]; intros eff_nm op0 v0 T eff Htype Hdc; simpl in *.

  - (* DC_Hole *)
    exact (perform_requires_effect _ _ _ _ _ _ Htype).

  - (* DC_AppFun *)
    remember (E_App (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) e2_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      apply lin_split_nil_inv in H as [HD1 HD2]. subst.
      eapply effect_in_union_right. eapply effect_in_union_left.
      exact (IHD eff_nm op0 v0 _ eff1 Htype1 Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_AppArg *)
    remember (E_App (value_to_expr v_) (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0)))) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      apply lin_split_nil_inv in H as [HD1 HD2]. subst.
      eapply effect_in_union_right. eapply effect_in_union_right.
      exact (IHD eff_nm op0 v0 _ eff2 Htype2 Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_Let *)
    remember (E_Let (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) e2_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      apply lin_split_nil_inv in H as [HD1 HD2]. subst.
      eapply effect_in_union_left.
      exact (IHD eff_nm op0 v0 _ eff1 Htype1 Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_Select *)
    remember (E_Select (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) l_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      exact (IHD eff_nm op0 v0 _ eff Htype Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_Annot *)
    remember (E_Annot (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) T_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      exact (IHD eff_nm op0 v0 _ eff Htype Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_PerformArg *)
    remember (E_Perform en_ opn_ (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0)))) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_ H3_. subst.
      eapply effect_in_union_right.
      exact (IHD eff_nm op0 v0 _ eff' Htype Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_RecordField *)
    remember (E_Record (done_ ++ (l_, plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) :: rest_)) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + (* T_Record *)
      injection Heqeform as Hfields. subst.
      destruct (rft_field_effect_incl _ _ _ _ _ _ _ _ _ H)
        as [A [eff_e [He Hsub_field]]].
      eapply effect_in_row_subset; [| exact Hsub_field].
      exact (IHD eff_nm op0 v0 _ _ He Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_HandleOther *)
    destruct Hdc as [Hno_clause Hdc'].
    destruct (has_type_handle_inv _ _ _ _ _ Htype)
      as [en_handle [ct [he [ce [Htye [Hwf [Hpass Hsub]]]]]]].
    assert (Hin_comp : effect_in_row eff_nm ce).
    { exact (IHD eff_nm op0 v0 _ _ Htye Hdc'). }
    (* Case split: does the handler handle eff_nm? *)
    destruct (String.string_dec eff_nm en_handle) as [Heq_en | Hneq_en].
    + (* eff_nm = en_handle: handler handles this effect.
         But dc_no_match says no clause handles eff_nm — contradiction. *)
      subst en_handle.
      (* From HWF: all ops of the effect have clauses *)
      inversion Hwf; subst.
      (* From the computation typing, get the perform *)
      destruct (delimited_context_typing _ _ _ _ _ Htye)
        as [A_hole [eff_hole [Hty_perf _]]].
      destruct (has_type_perform_inv _ _ _ _ _ _ Hty_perf)
        as [eff_sig' [arg_ty' [ret_ty' [ef' [Hl1 [Hl2 [_ [_ _]]]]]]]].
      (* Unify eff_sig' with eff_sig from HWF *)
      match goal with
      | [ Hlook : lookup_effect Sigma eff_nm = Some eff_sig |- _ ] =>
          rewrite Hl1 in Hlook; injection Hlook as <-
      end.
      (* lookup_op_In: the operation is in the signature *)
      pose proof (lookup_op_In _ _ _ _ Hl2) as Hop_in_sig.
      (* all_ops_handled: there exists a clause for this op *)
      destruct (H2 _ _ _ Hop_in_sig) as [e_body Hcl_in].
      (* But dc_no_match says every clause has en ≠ eff_nm *)
      specialize (Hno_clause _ Hcl_in). simpl in Hno_clause.
      exfalso. exact (Hno_clause eq_refl).
    + (* eff_nm ≠ en_handle: pass-through *)
      eapply effect_in_row_subset; [| exact Hsub].
      exact (Hpass eff_nm Hin_comp Hneq_en).

  - (* DC_ExtendVal *)
    remember (E_Extend l_ (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) e2_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_ H3_. subst.
      apply lin_split_nil_inv in H as [HD1 HD2]. subst.
      eapply effect_in_union_left.
      exact (IHD eff_nm op0 v0 _ eff1 Htype1 Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_ExtendRec *)
    remember (E_Extend l_ (value_to_expr v_) (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0)))) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_ H3_. subst.
      apply lin_split_nil_inv in H as [HD1 HD2]. subst.
      eapply effect_in_union_right.
      exact (IHD eff_nm op0 v0 _ eff2 Htype2 Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.

  - (* DC_Resume *)
    remember (E_Resume (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0)))) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_. subst.
      exact (IHD eff_nm op0 v0 _ eff Htype Hdc).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity; try exact IHD; try assumption.
Qed.
