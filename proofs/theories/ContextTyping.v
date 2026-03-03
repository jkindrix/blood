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
      as [en [ct [he [ce [Htye [Hwf Hsub]]]]]].
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

(** ** plug_delimited preserves effect containment

    If D[perform eff_nm.op(v)] is well-typed with effect eff,
    then eff_nm is in eff. *)

Lemma plug_delimited_perform_effect :
  forall Sigma D eff_nm op v T eff,
    has_type Sigma [] []
      (plug_delimited D (E_Perform eff_nm op (value_to_expr v))) T eff ->
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
  ]; intros eff_nm op0 v0 T eff Htype; simpl in *.

  - (* DC_Hole *)
    exact (perform_requires_effect _ _ _ _ _ _ Htype).

  - (* DC_AppFun *)
    remember (E_App (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) e2_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      apply lin_split_nil_inv in H as [HD1 HD2]. subst.
      eapply effect_in_union_right. eapply effect_in_union_left.
      exact (IHD eff_nm op0 v0 _ eff1 Htype1).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity. exact IHD.

  - (* DC_AppArg *)
    remember (E_App (value_to_expr v_) (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0)))) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      apply lin_split_nil_inv in H as [HD1 HD2]. subst.
      eapply effect_in_union_right. eapply effect_in_union_right.
      exact (IHD eff_nm op0 v0 _ eff2 Htype2).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity. exact IHD.

  - (* DC_Let *)
    remember (E_Let (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) e2_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      apply lin_split_nil_inv in H as [HD1 HD2]. subst.
      eapply effect_in_union_left.
      exact (IHD eff_nm op0 v0 _ eff1 Htype1).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity. exact IHD.

  - (* DC_Select *)
    remember (E_Select (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) l_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      exact (IHD eff_nm op0 v0 _ eff Htype).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity. exact IHD.

  - (* DC_Annot *)
    remember (E_Annot (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0))) T_) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_. subst.
      exact (IHD eff_nm op0 v0 _ eff Htype).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity. exact IHD.

  - (* DC_PerformArg *)
    remember (E_Perform en_ opn_ (plug_delimited D' (E_Perform eff_nm op0 (value_to_expr v0)))) as eform.
    remember (@nil ty) as Gamma. remember (@nil (linearity * bool)) as Delta.
    induction Htype; try discriminate.
    + injection Heqeform as H1_ H2_ H3_. subst.
      eapply effect_in_union_right.
      exact (IHD eff_nm op0 v0 _ eff' Htype).
    + subst. eapply effect_in_row_subset; [| eassumption].
      eapply IHHtype; try reflexivity. exact IHD.
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
  forall Sigma Gamma Delta clauses sig result_ty handler_eff
         eff_nm op_nm e_body,
    op_clauses_well_formed Sigma Gamma Delta clauses sig result_ty handler_eff ->
    In (OpClause eff_nm op_nm e_body) clauses ->
    exists arg_ty ret_ty,
      lookup_op sig op_nm = Some (arg_ty, ret_ty) /\
      has_type Sigma
               (arg_ty :: Ty_Arrow ret_ty result_ty handler_eff :: Gamma)
               ((Lin_Unrestricted, false) :: (Lin_Unrestricted, false) :: Delta)
               e_body result_ty handler_eff.
Proof.
  intros Sigma Gamma Delta clauses sig result_ty handler_eff
         eff_nm op_nm e_body Hwf Hin.
  induction Hwf.
  - (* OpClauses_Nil *) destruct Hin.
  - (* OpClauses_Cons *)
    destruct Hin as [Heq | Hin_rest].
    + (* Head match *)
      injection Heq as H1 H2 H3. subst.
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
Qed.
