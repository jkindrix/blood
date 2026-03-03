(** * Blood Core Calculus — Substitution

    This file defines substitution for de Bruijn indexed terms and
    proves the key substitution lemma: substitution preserves typing.

    Reference: FORMAL_SEMANTICS.md §7 (Progress and Preservation)
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
From Stdlib Require Import PeanoNat.
From Stdlib Require Import Lia.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Typing.

(** ** Shifting (de Bruijn)

    Shift free variables in an expression by [d] starting at cutoff [c].
    This is needed to avoid variable capture during substitution. *)

Fixpoint shift_expr (d : nat) (c : nat) (e : expr) : expr :=
  match e with
  | E_Var x =>
      if c <=? x then E_Var (x + d)
      else E_Var x
  | E_Const k => E_Const k
  | E_Lam T body =>
      E_Lam T (shift_expr d (S c) body)
  | E_App e1 e2 =>
      E_App (shift_expr d c e1) (shift_expr d c e2)
  | E_Let e1 e2 =>
      E_Let (shift_expr d c e1) (shift_expr d (S c) e2)
  | E_Annot e1 T =>
      E_Annot (shift_expr d c e1) T
  | E_Record fields =>
      E_Record (map (fun '(l, ei) => (l, shift_expr d c ei)) fields)
  | E_Select e1 l =>
      E_Select (shift_expr d c e1) l
  | E_Extend l e1 e2 =>
      E_Extend l (shift_expr d c e1) (shift_expr d c e2)
  | E_Perform eff op e1 =>
      E_Perform eff op (shift_expr d c e1)
  | E_Handle h e1 =>
      E_Handle (shift_handler d c h) (shift_expr d c e1)
  | E_Resume e1 =>
      E_Resume (shift_expr d c e1)
  end

with shift_handler (d : nat) (c : nat) (h : handler) : handler :=
  match h with
  | Handler hk e_ret clauses =>
      Handler hk
              (shift_expr d (S c) e_ret)  (** return binds one var *)
              (map (shift_op_clause d c) clauses)
  end

with shift_op_clause (d : nat) (c : nat) (cl : op_clause) : op_clause :=
  match cl with
  | OpClause eff op body =>
      OpClause eff op (shift_expr d (S (S c)) body)
      (** binds arg and resume *)
  end.

(** ** Substitution

    [subst j s e] substitutes expression [s] for variable [j] in [e]. *)

Fixpoint subst (j : nat) (s : expr) (e : expr) : expr :=
  match e with
  | E_Var x =>
      if j =? x then s
      else if j <? x then E_Var (x - 1)  (** shift down *)
      else E_Var x
  | E_Const k => E_Const k
  | E_Lam T body =>
      E_Lam T (subst (S j) (shift_expr 1 0 s) body)
  | E_App e1 e2 =>
      E_App (subst j s e1) (subst j s e2)
  | E_Let e1 e2 =>
      E_Let (subst j s e1) (subst (S j) (shift_expr 1 0 s) e2)
  | E_Annot e1 T =>
      E_Annot (subst j s e1) T
  | E_Record fields =>
      E_Record (map (fun '(l, ei) => (l, subst j s ei)) fields)
  | E_Select e1 l =>
      E_Select (subst j s e1) l
  | E_Extend l e1 e2 =>
      E_Extend l (subst j s e1) (subst j s e2)
  | E_Perform eff op e1 =>
      E_Perform eff op (subst j s e1)
  | E_Handle h e1 =>
      E_Handle (subst_handler j s h) (subst j s e1)
  | E_Resume e1 =>
      E_Resume (subst j s e1)
  end

with subst_handler (j : nat) (s : expr) (h : handler) : handler :=
  match h with
  | Handler hk e_ret clauses =>
      Handler hk
              (subst (S j) (shift_expr 1 0 s) e_ret)
              (map (subst_op_clause j s) clauses)
  end

with subst_op_clause (j : nat) (s : expr) (cl : op_clause) : op_clause :=
  match cl with
  | OpClause eff op body =>
      OpClause eff op
               (subst (S (S j)) (shift_expr 2 0 s) body)
  end.

(** ** Notation for substitution *)

Notation "e [ j ':=' s ]" := (subst j s e) (at level 20, left associativity).

(** ** Context removal

    Remove the [j]-th element from a context. *)

Fixpoint remove_nth {A : Type} (j : nat) (l : list A) : list A :=
  match j, l with
  | 0, _ :: rest => rest
  | S n, x :: rest => x :: remove_nth n rest
  | _, [] => []
  end.

(** ** Context insertion *)

Definition insert_at {A : Type} (n : nat) (x : A) (l : list A) : list A :=
  firstn n l ++ x :: skipn n l.

Lemma insert_at_0 : forall A (x : A) (l : list A),
  insert_at 0 x l = x :: l.
Proof. reflexivity. Qed.

Lemma insert_at_S_cons : forall A n (x a : A) (l : list A),
  insert_at (S n) x (a :: l) = a :: insert_at n x l.
Proof. intros. unfold insert_at. simpl. reflexivity. Qed.

(** ** Lookup in inserted context *)

Lemma lookup_var_insert_ge : forall n Gamma x T U,
  n <= x ->
  lookup_var Gamma x = Some T ->
  lookup_var (insert_at n U Gamma) (Datatypes.S x) = Some T.
Proof.
  induction n; intros Gamma x T U Hle Hlook.
  - simpl. exact Hlook.
  - destruct Gamma as [| A rest].
    + destruct x; simpl in Hlook; discriminate.
    + destruct x as [| x'].
      * lia.
      * simpl in Hlook. rewrite insert_at_S_cons. simpl.
        apply IHn; [lia | exact Hlook].
Qed.

Lemma lookup_var_insert_lt : forall n Gamma x T U,
  x < n ->
  lookup_var Gamma x = Some T ->
  lookup_var (insert_at n U Gamma) x = Some T.
Proof.
  induction n; intros Gamma x T U Hlt Hlook.
  - lia.
  - destruct Gamma as [| A rest].
    + destruct x; simpl in Hlook; discriminate.
    + rewrite insert_at_S_cons.
      destruct x as [| x'].
      * simpl. simpl in Hlook. exact Hlook.
      * simpl. simpl in Hlook. apply IHn; [lia | exact Hlook].
Qed.

(** ** Linearity split with unrestricted insertion *)

Lemma lin_split_insert : forall Delta Delta1 Delta2 n,
  lin_split Delta Delta1 Delta2 ->
  lin_split (insert_at n (Lin_Unrestricted, false) Delta)
            (insert_at n (Lin_Unrestricted, false) Delta1)
            (insert_at n (Lin_Unrestricted, false) Delta2).
Proof.
  intros Delta Delta1 Delta2 n Hsplit.
  generalize dependent n.
  induction Hsplit; intro n.
  - (* Split_Nil *)
    destruct n; simpl; apply Split_Unrestricted; apply Split_Nil.
  - (* Split_Unrestricted *)
    destruct n.
    + simpl. apply Split_Unrestricted. apply Split_Unrestricted. exact Hsplit.
    + rewrite !insert_at_S_cons.
      apply Split_Unrestricted. apply IHHsplit.
  - (* Split_Linear_Left *)
    destruct n.
    + simpl. apply Split_Unrestricted. apply Split_Linear_Left. exact Hsplit.
    + rewrite !insert_at_S_cons.
      apply Split_Linear_Left. apply IHHsplit.
  - (* Split_Linear_Right *)
    destruct n.
    + simpl. apply Split_Unrestricted. apply Split_Linear_Right. exact Hsplit.
    + rewrite !insert_at_S_cons.
      apply Split_Linear_Right. apply IHHsplit.
  - (* Split_Affine_Left *)
    destruct n.
    + simpl. apply Split_Unrestricted. apply Split_Affine_Left. exact Hsplit.
    + rewrite !insert_at_S_cons.
      apply Split_Affine_Left. apply IHHsplit.
  - (* Split_Affine_Right *)
    destruct n.
    + simpl. apply Split_Unrestricted. apply Split_Affine_Right. exact Hsplit.
    + rewrite !insert_at_S_cons.
      apply Split_Affine_Right. apply IHHsplit.
  - (* Split_Affine_Neither *)
    destruct n.
    + simpl. apply Split_Unrestricted. apply Split_Affine_Neither. exact Hsplit.
    + rewrite !insert_at_S_cons.
      apply Split_Affine_Neither. apply IHHsplit.
Qed.

(** ** Mutual induction scheme for typing judgment *)

Scheme has_type_mut_ind := Induction for has_type Sort Prop
  with handler_wf_mut_ind := Induction for handler_well_formed Sort Prop
  with op_clauses_wf_mut_ind := Induction for op_clauses_well_formed Sort Prop
  with record_fields_typed_mut_ind := Induction for record_fields_typed Sort Prop.

(** ** Weakening Lemma

    If Γ ⊢ e : T / ε then (Γ with extra binding at position n) ⊢ (shifted e) : T / ε *)

Lemma weakening :
  forall Sigma Gamma Delta e T eff,
    has_type Sigma Gamma Delta e T eff ->
    forall U n,
    has_type Sigma
             (insert_at n U Gamma)
             (insert_at n (Lin_Unrestricted, false) Delta)
             (shift_expr 1 n e) T eff.
Proof.
  apply (has_type_mut_ind
    (fun Sigma Gamma Delta e T eff _ =>
       forall U n,
       has_type Sigma (insert_at n U Gamma)
                      (insert_at n (Lin_Unrestricted, false) Delta)
                      (shift_expr 1 n e) T eff)
    (fun Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff _ =>
       forall U n,
       handler_well_formed Sigma (insert_at n U Gamma)
                                 (insert_at n (Lin_Unrestricted, false) Delta)
                                 (shift_handler 1 n h)
                                 eff_name comp_ty result_ty handler_eff)
    (fun Sigma Gamma Delta clauses eff_sig result_ty handler_eff _ =>
       forall U n,
       op_clauses_well_formed Sigma (insert_at n U Gamma)
                                    (insert_at n (Lin_Unrestricted, false) Delta)
                                    (map (shift_op_clause 1 n) clauses)
                                    eff_sig result_ty handler_eff)
    (fun Sigma Gamma Delta fields field_types eff _ =>
       forall U n,
       record_fields_typed Sigma (insert_at n U Gamma)
                                 (insert_at n (Lin_Unrestricted, false) Delta)
                                 (map (fun '(l, e) => (l, shift_expr 1 n e)) fields)
                                 field_types eff)).

  - (* T_Var *)
    intros Sigma Gamma Delta x T Hlook U n. simpl.
    destruct (n <=? x) eqn:Hnx.
    + replace (x + 1) with (Datatypes.S x) by lia.
      apply T_Var. apply lookup_var_insert_ge; [apply Nat.leb_le; exact Hnx | exact Hlook].
    + apply T_Var. apply lookup_var_insert_lt; [apply Nat.leb_gt; exact Hnx | exact Hlook].

  - (* T_Const *)
    intros Sigma Gamma Delta c U n. simpl. apply T_Const.

  - (* T_Lam *)
    intros Sigma Gamma Delta A B eff body Hbody IHbody U n. simpl.
    apply T_Lam.
    specialize (IHbody U (Datatypes.S n)).
    rewrite insert_at_S_cons in IHbody.
    rewrite insert_at_S_cons in IHbody.
    exact IHbody.

  - (* T_App *)
    intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B fn_eff eff1 eff2
           Hsplit He1 IH1 He2 IH2 U n. simpl.
    eapply T_App.
    + apply lin_split_insert. exact Hsplit.
    + apply IH1.
    + apply IH2.

  - (* T_Let *)
    intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B eff1 eff2
           Hsplit He1 IH1 He2 IH2 U n. simpl.
    eapply T_Let.
    + apply lin_split_insert. exact Hsplit.
    + apply IH1.
    + specialize (IH2 U (Datatypes.S n)).
      rewrite insert_at_S_cons in IH2.
      rewrite insert_at_S_cons in IH2.
      exact IH2.

  - (* T_Annot *)
    intros Sigma Gamma Delta e T eff He IHe U n. simpl.
    apply T_Annot. apply IHe.

  - (* T_Record *)
    intros Sigma Gamma Delta fields field_types eff Hfields IHfields U n. simpl.
    apply T_Record. apply IHfields.

  - (* T_Select *)
    intros Sigma Gamma Delta e l T fields eff He IHe Hlook U n. simpl.
    eapply T_Select.
    + apply IHe.
    + exact Hlook.

  - (* T_Perform *)
    intros Sigma Gamma Delta e eff_name op eff_sig arg_ty ret_ty eff'
           Hlookeff Hlookop He IHe U n. simpl.
    eapply T_Perform; [exact Hlookeff | exact Hlookop | apply IHe].

  - (* T_Handle *)
    intros Sigma Gamma Delta Delta1 Delta2 h e eff_name comp_ty result_ty
           handler_eff comp_eff Hsplit He IHe Hh IHh U n. simpl.
    eapply T_Handle.
    + apply lin_split_insert. exact Hsplit.
    + apply IHe.
    + apply IHh.

  - (* T_Sub *)
    intros Sigma Gamma Delta e T eff eff' He IHe Hsub U n.
    apply T_Sub with (eff := eff).
    + apply IHe.
    + exact Hsub.

  - (* HWF — Handler well-formed *)
    intros Sigma Gamma Delta hk e_ret clauses eff_name comp_ty result_ty
           handler_eff eff_sig Hlookeff Hret IHret Hclauses IHclauses U n.
    simpl.
    eapply HWF.
    + exact Hlookeff.
    + specialize (IHret U (Datatypes.S n)).
      rewrite insert_at_S_cons in IHret.
      rewrite insert_at_S_cons in IHret.
      exact IHret.
    + apply IHclauses.

  - (* OpClauses_Nil *)
    intros Sigma Gamma Delta sig result_ty eff U n.
    simpl. apply OpClauses_Nil.

  - (* OpClauses_Cons *)
    intros Sigma Gamma Delta eff_nm op_nm e_body rest sig result_ty
           handler_eff arg_ty ret_ty Hlookop Hbody IHbody Hrest IHrest U n.
    simpl. eapply OpClauses_Cons.
    + exact Hlookop.
    + specialize (IHbody U (Datatypes.S (Datatypes.S n))).
      rewrite !insert_at_S_cons in IHbody.
      exact IHbody.
    + apply IHrest.

  - (* RFT_Nil *)
    intros Sigma Gamma Delta U n.
    simpl. apply RFT_Nil.

  - (* RFT_Cons *)
    intros Sigma Gamma Delta l e T rest_e rest_t eff1 eff2
           He IHe Hrest IHrest U n.
    simpl. apply RFT_Cons.
    + apply IHe.
    + apply IHrest.
Qed.

(** ** Any linearity context can be split *)

Lemma lin_split_exists : forall Delta,
  exists Delta1 Delta2, lin_split Delta Delta1 Delta2.
Proof.
  induction Delta as [| [lm used] rest IH].
  - exists [], []. apply Split_Nil.
  - destruct IH as [D1 [D2 Hsplit]].
    destruct lm.
    + (* Lin_Unrestricted *)
      exists ((Lin_Unrestricted, used) :: D1), ((Lin_Unrestricted, used) :: D2).
      apply Split_Unrestricted. exact Hsplit.
    + (* Lin_Linear *)
      exists ((Lin_Linear, used) :: D1), ((Lin_Linear, true) :: D2).
      apply Split_Linear_Left. exact Hsplit.
    + (* Lin_Affine *)
      exists ((Lin_Affine, used) :: D1), ((Lin_Affine, true) :: D2).
      apply Split_Affine_Left. exact Hsplit.
Qed.

(** ** Typing is independent of the linearity context

    Since [T_Var] and [T_Const] do not inspect Delta, and Delta is only
    threaded through [lin_split], we can freely change the linearity context
    in any typing derivation. *)

Lemma has_type_lin_irrelevant :
  forall Sigma Gamma Delta e T eff,
    has_type Sigma Gamma Delta e T eff ->
    forall Delta',
    has_type Sigma Gamma Delta' e T eff.
Proof.
  apply (has_type_mut_ind
    (fun Sigma Gamma Delta e T eff _ =>
       forall Delta', has_type Sigma Gamma Delta' e T eff)
    (fun Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff _ =>
       forall Delta',
       handler_well_formed Sigma Gamma Delta' h
                           eff_name comp_ty result_ty handler_eff)
    (fun Sigma Gamma Delta clauses eff_sig result_ty handler_eff _ =>
       forall Delta',
       op_clauses_well_formed Sigma Gamma Delta' clauses
                              eff_sig result_ty handler_eff)
    (fun Sigma Gamma Delta fields field_types eff _ =>
       forall Delta',
       record_fields_typed Sigma Gamma Delta' fields field_types eff)).

  - (* T_Var *) intros. apply T_Var. assumption.
  - (* T_Const *) intros. apply T_Const.
  - (* T_Lam *)
    intros Sigma Gamma Delta A B eff body _ IH Delta'.
    apply T_Lam. apply (IH ((Lin_Unrestricted, false) :: Delta')).
  - (* T_App *)
    intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B fn_eff eff1 eff2
           _ _ IH1 _ IH2 Delta'.
    destruct (lin_split_exists Delta') as [D1' [D2' Hsplit']].
    eapply T_App; [exact Hsplit' | apply IH1 | apply IH2].
  - (* T_Let *)
    intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B eff1 eff2
           _ _ IH1 _ IH2 Delta'.
    destruct (lin_split_exists Delta') as [D1' [D2' Hsplit']].
    eapply T_Let.
    + exact Hsplit'.
    + apply IH1.
    + apply (IH2 ((Lin_Unrestricted, false) :: D2')).
  - (* T_Annot *) intros. apply T_Annot. auto.
  - (* T_Record *) intros. apply T_Record. auto.
  - (* T_Select *) intros. eapply T_Select; eauto.
  - (* T_Perform *) intros. eapply T_Perform; eauto.
  - (* T_Handle *)
    intros Sigma Gamma Delta Delta1 Delta2 h e eff_name comp_ty result_ty
           handler_eff comp_eff _ _ IHe _ IHh Delta'.
    destruct (lin_split_exists Delta') as [D1' [D2' Hsplit']].
    eapply T_Handle; [exact Hsplit' | apply IHe | apply IHh].
  - (* T_Sub *)
    intros Sigma Gamma Delta e T eff eff' _ IH Hsub Delta'.
    apply T_Sub with (eff := eff); [apply IH | exact Hsub].
  - (* HWF *)
    intros Sigma Gamma Delta hk e_ret clauses eff_name comp_ty result_ty
           handler_eff eff_sig Hlook _ IHret _ IHclauses Delta'.
    eapply HWF.
    + exact Hlook.
    + apply (IHret ((Lin_Unrestricted, false) :: Delta')).
    + apply IHclauses.
  - (* OpClauses_Nil *) intros. apply OpClauses_Nil.
  - (* OpClauses_Cons *)
    intros Sigma Gamma Delta eff_nm op_nm e_body rest sig result_ty
           handler_eff arg_ty ret_ty Hlookop _ IHbody _ IHrest Delta'.
    eapply OpClauses_Cons.
    + exact Hlookop.
    + apply (IHbody ((Lin_Unrestricted, false) :: (Lin_Unrestricted, false) :: Delta')).
    + apply IHrest.
  - (* RFT_Nil *) intros. apply RFT_Nil.
  - (* RFT_Cons *)
    intros Sigma Gamma Delta l e T rest_e rest_t eff1 eff2
           _ IHe _ IHrest Delta'.
    apply RFT_Cons; [apply IHe | apply IHrest].
Qed.

Lemma op_clauses_wf_lin_irrelevant :
  forall Sigma Gamma Delta clauses sig result_ty handler_eff,
    op_clauses_well_formed Sigma Gamma Delta clauses sig result_ty handler_eff ->
    forall Delta',
    op_clauses_well_formed Sigma Gamma Delta' clauses sig result_ty handler_eff.
Proof.
  intros Sigma Gamma Delta clauses sig result_ty handler_eff Hcl.
  induction Hcl; intros Delta'.
  - apply OpClauses_Nil.
  - eapply OpClauses_Cons.
    + eassumption.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ H0
               ((Lin_Unrestricted, false) :: (Lin_Unrestricted, false) :: Delta')).
    + apply IHHcl.
Qed.

Lemma handler_wf_lin_irrelevant :
  forall Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff,
    handler_well_formed Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff ->
    forall Delta',
    handler_well_formed Sigma Gamma Delta' h eff_name comp_ty result_ty handler_eff.
Proof.
  intros Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff Hwf Delta'.
  inversion Hwf; subst.
  eapply HWF.
  - eassumption.
  - apply (has_type_lin_irrelevant _ _ _ _ _ _ H0
             ((Lin_Unrestricted, false) :: Delta')).
  - apply (op_clauses_wf_lin_irrelevant _ _ _ _ _ _ _ H1 Delta').
Qed.

(** ** Lookup in context with element removed *)

Lemma lookup_var_remove_gt : forall j Gamma x T,
  j < x ->
  lookup_var Gamma x = Some T ->
  lookup_var (remove_nth j Gamma) (x - 1) = Some T.
Proof.
  induction j; intros Gamma x T Hlt Hlook.
  - destruct Gamma as [| A rest]; [destruct x; simpl in Hlook; discriminate |].
    destruct x as [| x']; [lia |]. simpl. simpl in Hlook.
    replace (x' - 0) with x' by lia. exact Hlook.
  - destruct Gamma as [| A rest]; [destruct x; simpl in Hlook; discriminate |].
    destruct x as [| x']; [lia |].
    simpl. simpl in Hlook.
    replace (Datatypes.S x' - 1) with x' by lia.
    destruct x' as [| x''].
    + lia.
    + simpl.
      assert (Hih := IHj rest (Datatypes.S x'') T ltac:(lia) Hlook).
      simpl in Hih. replace (x'' - 0) with x'' in Hih by lia. exact Hih.
Qed.

Lemma lookup_var_remove_lt : forall j Gamma x T,
  x < j ->
  lookup_var Gamma x = Some T ->
  lookup_var (remove_nth j Gamma) x = Some T.
Proof.
  induction j; intros Gamma x T Hlt Hlook.
  - lia.
  - destruct Gamma as [| A rest]; [destruct x; discriminate |].
    simpl. destruct x as [| x'].
    + simpl. simpl in Hlook. exact Hlook.
    + simpl. simpl in Hlook. apply IHj; [lia | exact Hlook].
Qed.

(** lookup_var is equivalent to nth_error on type_context *)

Lemma lookup_var_nth_error : forall Gamma x,
  lookup_var Gamma x = nth_error Gamma x.
Proof.
  induction Gamma as [| A rest IH]; intros x.
  - destruct x; reflexivity.
  - destruct x; simpl; [reflexivity | apply IH].
Qed.

(** Shift composition: shifting by d1 then d2 at the same cutoff = shifting by d1+d2 *)

Lemma shift_compose :
  forall e d1 d2 c,
    shift_expr d1 c (shift_expr d2 c e) = shift_expr (d1 + d2) c e.
Proof.
  intro e.
  apply (expr_nested_ind
    (fun e => forall d1 d2 c,
       shift_expr d1 c (shift_expr d2 c e) = shift_expr (d1 + d2) c e)
    (fun h => forall d1 d2 c,
       shift_handler d1 c (shift_handler d2 c h) = shift_handler (d1 + d2) c h)
    (fun cl => forall d1 d2 c,
       shift_op_clause d1 c (shift_op_clause d2 c cl) = shift_op_clause (d1 + d2) c cl)).
  - (* E_Var *) intros x d1 d2 c. simpl.
    destruct (Nat.leb c x) eqn:Hcx; simpl.
    + apply Nat.leb_le in Hcx.
      destruct (Nat.leb c (x + d2)) eqn:Hcxd; simpl.
      * f_equal. lia.
      * apply Nat.leb_gt in Hcxd. lia.
    + apply Nat.leb_gt in Hcx.
      destruct (Nat.leb c x) eqn:Hcx2; simpl.
      * apply Nat.leb_le in Hcx2. lia.
      * reflexivity.
  - (* E_Const *) intros co d1 d2 c. reflexivity.
  - (* E_Lam *) intros T body IH d1 d2 c. simpl. f_equal. apply IH.
  - (* E_App *) intros e1 e2 IH1 IH2 d1 d2 c. simpl. f_equal; [apply IH1 | apply IH2].
  - (* E_Let *) intros e1 e2 IH1 IH2 d1 d2 c. simpl. f_equal; [apply IH1 | apply IH2].
  - (* E_Annot *) intros e' T IH d1 d2 c. simpl. f_equal. apply IH.
  - (* E_Record *) intros fields HFA d1 d2 c. simpl. f_equal.
    induction HFA as [| [l ei] rest Hei IHrest]; [reflexivity |].
    simpl. f_equal; [f_equal; apply Hei | exact IHIHrest].
  - (* E_Select *) intros e' l IH d1 d2 c. simpl. f_equal. apply IH.
  - (* E_Extend *) intros l e1 e2 IH1 IH2 d1 d2 c. simpl. f_equal; [apply IH1 | apply IH2].
  - (* E_Perform *) intros eff op e' IH d1 d2 c. simpl. f_equal. apply IH.
  - (* E_Handle *) intros h' e' IHh IHe d1 d2 c. simpl. f_equal; [apply IHh | apply IHe].
  - (* E_Resume *) intros e' IH d1 d2 c. simpl. f_equal. apply IH.
  - (* Handler *) intros hk e_ret clauses IHret IHclauses d1 d2 c. simpl. f_equal.
    + apply IHret.
    + induction IHclauses as [| cl rest Hcl IHrest]; [reflexivity |].
      simpl. f_equal; [apply Hcl | exact IHIHrest].
  - (* OpClause *) intros eff op body IH d1 d2 c. simpl. f_equal. apply IH.
Qed.

(** Weakening at position 0, expressed with cons instead of insert_at *)

Lemma weakening_cons :
  forall Sigma Gamma Delta e T eff W,
    has_type Sigma Gamma Delta e T eff ->
    has_type Sigma (W :: Gamma)
             ((Lin_Unrestricted, false) :: Delta)
             (shift_expr 1 0 e) T eff.
Proof.
  intros Sigma Gamma Delta e T eff W Hty.
  pose proof (weakening _ _ _ _ _ _ Hty W 0) as H.
  rewrite insert_at_0 in H. simpl in H. exact H.
Qed.

(** ** Substitution Preserves Typing (generalized)

    This is THE key lemma for type preservation.
    Reference: FORMAL_SEMANTICS.md §12.1

    Generalized to substitution at arbitrary index [j], which is required
    because going under binders shifts the substitution index. *)

Theorem subst_preserves_typing :
  forall Sigma Gamma Delta e T eff,
    has_type Sigma Gamma Delta e T eff ->
    forall j v Sty,
    lookup_var Gamma j = Some Sty ->
    has_type Sigma (remove_nth j Gamma) (remove_nth j Delta) v Sty Eff_Pure ->
    has_type Sigma (remove_nth j Gamma) (remove_nth j Delta) (subst j v e) T eff.
Proof.
  apply (has_type_mut_ind
    (fun Sigma Gamma Delta e T eff _ =>
       forall j v Sty,
       lookup_var Gamma j = Some Sty ->
       has_type Sigma (remove_nth j Gamma) (remove_nth j Delta) v Sty Eff_Pure ->
       has_type Sigma (remove_nth j Gamma) (remove_nth j Delta)
                      (subst j v e) T eff)
    (fun Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff _ =>
       forall j v Sty,
       lookup_var Gamma j = Some Sty ->
       has_type Sigma (remove_nth j Gamma) (remove_nth j Delta) v Sty Eff_Pure ->
       handler_well_formed Sigma (remove_nth j Gamma) (remove_nth j Delta)
                                 (subst_handler j v h)
                                 eff_name comp_ty result_ty handler_eff)
    (fun Sigma Gamma Delta clauses eff_sig result_ty handler_eff _ =>
       forall j v Sty,
       lookup_var Gamma j = Some Sty ->
       has_type Sigma (remove_nth j Gamma) (remove_nth j Delta) v Sty Eff_Pure ->
       op_clauses_well_formed Sigma (remove_nth j Gamma) (remove_nth j Delta)
                                    (map (subst_op_clause j v) clauses)
                                    eff_sig result_ty handler_eff)
    (fun Sigma Gamma Delta fields field_types eff _ =>
       forall j v Sty,
       lookup_var Gamma j = Some Sty ->
       has_type Sigma (remove_nth j Gamma) (remove_nth j Delta) v Sty Eff_Pure ->
       record_fields_typed Sigma (remove_nth j Gamma) (remove_nth j Delta)
                                 (map (fun '(l, e) => (l, subst j v e)) fields)
                                 field_types eff)).

  - (* T_Var *)
    intros Sigma Gamma Delta x T Hlook j v Sty HnthSty Hval. simpl.
    destruct (j =? x) eqn:Hjx.
    + (* j = x: substitute *)
      apply Nat.eqb_eq in Hjx. subst x.
      rewrite Hlook in HnthSty. injection HnthSty as <-.
      exact Hval.
    + destruct (j <? x) eqn:Hjx'.
      * (* j < x: shift down *)
        apply Nat.ltb_lt in Hjx'. apply T_Var.
        apply lookup_var_remove_gt; [exact Hjx' | exact Hlook].
      * (* j > x: unchanged *)
        apply Nat.ltb_ge in Hjx'. apply Nat.eqb_neq in Hjx.
        assert (j > x) by lia. apply T_Var.
        apply lookup_var_remove_lt; [lia | exact Hlook].

  - (* T_Const *)
    intros Sigma Gamma Delta c j v Sty HnthSty Hval. simpl. apply T_Const.

  - (* T_Lam *)
    intros Sigma Gamma Delta A B eff body _ IH j v Sty HnthSty Hval. simpl.
    apply T_Lam.
    apply (IH (Datatypes.S j) (shift_expr 1 0 v) Sty).
    + simpl. exact HnthSty.
    + simpl. apply weakening_cons. exact Hval.

  - (* T_App *)
    intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B fn_eff eff1 eff2
           Hsplit _ IH1 _ IH2 j v Sty HnthSty Hval. simpl.
    destruct (lin_split_exists (remove_nth j Delta)) as [D1' [D2' Hsplit']].
    eapply T_App.
    + exact Hsplit'.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ (IH1 j v Sty HnthSty
        (has_type_lin_irrelevant _ _ _ _ _ _ Hval (remove_nth j Delta1)))).
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ (IH2 j v Sty HnthSty
        (has_type_lin_irrelevant _ _ _ _ _ _ Hval (remove_nth j Delta2)))).

  - (* T_Let *)
    intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B eff1 eff2
           Hsplit _ IH1 _ IH2 j v Sty HnthSty Hval. simpl.
    destruct (lin_split_exists (remove_nth j Delta)) as [D1' [D2' Hsplit']].
    eapply T_Let.
    + exact Hsplit'.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ (IH1 j v Sty HnthSty
        (has_type_lin_irrelevant _ _ _ _ _ _ Hval (remove_nth j Delta1)))).
    + apply (has_type_lin_irrelevant _ _ _ _ _ _
        (IH2 (Datatypes.S j) (shift_expr 1 0 v) Sty
          ltac:(simpl; exact HnthSty)
          ltac:(simpl; apply weakening_cons;
                apply has_type_lin_irrelevant with (Delta := remove_nth j Delta);
                exact Hval))
        ((Lin_Unrestricted, false) :: D2')).

  - (* T_Annot *)
    intros Sigma Gamma Delta e T eff _ IH j v Sty HnthSty Hval. simpl.
    apply T_Annot. exact (IH j v Sty HnthSty Hval).

  - (* T_Record *)
    intros Sigma Gamma Delta fields field_types eff _ IH j v Sty HnthSty Hval. simpl.
    apply T_Record. exact (IH j v Sty HnthSty Hval).

  - (* T_Select *)
    intros Sigma Gamma Delta e l T fields eff _ IH Hlookf j v Sty HnthSty Hval. simpl.
    eapply T_Select; [exact (IH j v Sty HnthSty Hval) | exact Hlookf].

  - (* T_Perform *)
    intros Sigma Gamma Delta e eff_name op eff_sig arg_ty ret_ty eff'
           Hlookeff Hlookop _ IH j v Sty HnthSty Hval. simpl.
    eapply T_Perform; [exact Hlookeff | exact Hlookop | exact (IH j v Sty HnthSty Hval)].

  - (* T_Handle *)
    intros Sigma Gamma Delta Delta1 Delta2 h e eff_name comp_ty result_ty
           handler_eff comp_eff Hsplit _ IHe _ IHh j v Sty HnthSty Hval. simpl.
    destruct (lin_split_exists (remove_nth j Delta)) as [D1' [D2' Hsplit']].
    eapply T_Handle.
    + exact Hsplit'.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _ (IHe j v Sty HnthSty
        (has_type_lin_irrelevant _ _ _ _ _ _ Hval (remove_nth j Delta1)))).
    + apply (handler_wf_lin_irrelevant _ _ _ _ _ _ _ _
        (IHh j v Sty HnthSty
          (has_type_lin_irrelevant _ _ _ _ _ _ Hval (remove_nth j Delta2)))
        D2').

  - (* T_Sub *)
    intros Sigma Gamma Delta e T eff eff' _ IH Hsub j v Sty HnthSty Hval.
    apply T_Sub with (eff := eff); [exact (IH j v Sty HnthSty Hval) | exact Hsub].

  - (* HWF *)
    intros Sigma Gamma Delta hk e_ret clauses eff_name comp_ty result_ty
           handler_eff eff_sig Hlookeff _ IHret _ IHclauses j v Sty HnthSty Hval.
    simpl. eapply HWF.
    + exact Hlookeff.
    + apply (has_type_lin_irrelevant _ _ _ _ _ _
        (IHret (Datatypes.S j) (shift_expr 1 0 v) Sty
          ltac:(simpl; exact HnthSty)
          ltac:(simpl; apply weakening_cons; exact Hval))
        ((Lin_Unrestricted, false) :: remove_nth j Delta)).
    + exact (IHclauses j v Sty HnthSty Hval).

  - (* OpClauses_Nil *)
    intros Sigma Gamma Delta sig result_ty eff j v Sty HnthSty Hval.
    simpl. apply OpClauses_Nil.

  - (* OpClauses_Cons *)
    intros Sigma Gamma Delta eff_nm op_nm e_body rest sig result_ty
           handler_eff arg_ty ret_ty Hlookop _ IHbody _ IHrest j v Sty HnthSty Hval.
    simpl. eapply OpClauses_Cons.
    + exact Hlookop.
    + assert (Hval2 : has_type Sigma
                (arg_ty :: Ty_Arrow ret_ty result_ty handler_eff :: remove_nth j Gamma)
                ((Lin_Unrestricted, false) :: (Lin_Unrestricted, false) :: remove_nth j Delta)
                (shift_expr 2 0 v) Sty Eff_Pure).
      { rewrite <- (shift_compose v 1 1).
        apply weakening_cons. apply weakening_cons. exact Hval. }
      apply (has_type_lin_irrelevant _ _ _ _ _ _
        (IHbody (Datatypes.S (Datatypes.S j)) (shift_expr 2 0 v) Sty
          ltac:(simpl; exact HnthSty)
          ltac:(simpl; exact Hval2))
        ((Lin_Unrestricted, false) :: (Lin_Unrestricted, false) :: remove_nth j Delta)).
    + exact (IHrest j v Sty HnthSty Hval).

  - (* RFT_Nil *)
    intros Sigma Gamma Delta j v Sty HnthSty Hval. simpl. apply RFT_Nil.

  - (* RFT_Cons *)
    intros Sigma Gamma Delta l e T rest_e rest_t eff1 eff2
           _ IHe _ IHrest j v Sty HnthSty Hval.
    simpl. apply RFT_Cons; [exact (IHe j v Sty HnthSty Hval) | exact (IHrest j v Sty HnthSty Hval)].
Qed.

(** ** Substitution at index 0 (corollary) *)

Theorem substitution_preserves_typing :
  forall Sigma Gamma Delta e v T U eff,
    has_type Sigma (U :: Gamma) ((Lin_Unrestricted, false) :: Delta) e T eff ->
    has_type Sigma Gamma Delta v U Eff_Pure ->
    has_type Sigma Gamma Delta (subst 0 v e) T eff.
Proof.
  intros Sigma Gamma Delta e v T U eff Htype Hval.
  apply (subst_preserves_typing _ _ _ _ _ _ Htype 0 v U).
  - reflexivity.
  - simpl. exact Hval.
Qed.

(** ** Multi-substitution

    Substitute multiple values simultaneously. Used for handler clause
    instantiation. *)

Fixpoint multi_subst (vals : list expr) (e : expr) : expr :=
  match vals with
  | [] => e
  | v :: rest =>
      multi_subst rest (subst 0 v e)
  end.

(** ** Shifting commutes with shifting (disjoint cutoffs)

    When c1 <= c2, shifting by d1 at c1 then by d2 at c2+d1
    equals shifting by d2 at c2 then by d1 at c1. *)

Lemma shift_shift_commute :
  forall e d1 d2 c1 c2,
    c1 <= c2 ->
    shift_expr d2 (c2 + d1) (shift_expr d1 c1 e) =
    shift_expr d1 c1 (shift_expr d2 c2 e).
Proof.
  intro e.
  apply (expr_nested_ind
    (fun e => forall d1 d2 c1 c2, c1 <= c2 ->
       shift_expr d2 (c2 + d1) (shift_expr d1 c1 e) =
       shift_expr d1 c1 (shift_expr d2 c2 e))
    (fun h => forall d1 d2 c1 c2, c1 <= c2 ->
       shift_handler d2 (c2 + d1) (shift_handler d1 c1 h) =
       shift_handler d1 c1 (shift_handler d2 c2 h))
    (fun cl => forall d1 d2 c1 c2, c1 <= c2 ->
       shift_op_clause d2 (c2 + d1) (shift_op_clause d1 c1 cl) =
       shift_op_clause d1 c1 (shift_op_clause d2 c2 cl))).

  - (* E_Var *)
    intros v d1 d2 c1 c2 Hle. simpl.
    destruct (c1 <=? v) eqn:Hc1v; destruct (c2 <=? v) eqn:Hc2v; simpl.
    + (* c1 <= v, c2 <= v *)
      destruct (c2 + d1 <=? v + d1) eqn:H1; destruct (c1 <=? v + d2) eqn:H2.
      * f_equal. apply Nat.leb_le in Hc1v. lia.
      * exfalso. apply Nat.leb_le in Hc1v. apply Nat.leb_gt in H2. lia.
      * exfalso. apply Nat.leb_le in Hc2v. apply Nat.leb_gt in H1. lia.
      * exfalso. apply Nat.leb_le in Hc2v. apply Nat.leb_gt in H1. lia.
    + (* c1 <= v, c2 > v *)
      rewrite Hc1v. simpl.
      destruct (c2 + d1 <=? v + d1) eqn:H1.
      * exfalso. apply Nat.leb_le in H1. apply Nat.leb_gt in Hc2v. lia.
      * reflexivity.
    + (* c1 > v, c2 <= v: impossible *)
      exfalso. apply Nat.leb_gt in Hc1v. apply Nat.leb_le in Hc2v. lia.
    + (* c1 > v, c2 > v *)
      rewrite Hc1v. simpl.
      destruct (c2 + d1 <=? v) eqn:H1.
      * exfalso. apply Nat.leb_le in H1. apply Nat.leb_gt in Hc2v. lia.
      * reflexivity.

  - (* E_Const *) intros c d1 d2 c1 c2 Hle. reflexivity.

  - (* E_Lam *) intros T body IH d1 d2 c1 c2 Hle. simpl. f_equal.
    apply (IH d1 d2 (S c1) (S c2)). lia.

  - (* E_App *) intros e1 e2 IH1 IH2 d1 d2 c1 c2 Hle. simpl.
    f_equal; [apply IH1 | apply IH2]; assumption.

  - (* E_Let *) intros e1 e2 IH1 IH2 d1 d2 c1 c2 Hle. simpl.
    f_equal; [apply IH1; assumption | apply (IH2 d1 d2 (S c1) (S c2)); lia].

  - (* E_Annot *) intros e0 T IH d1 d2 c1 c2 Hle. simpl.
    f_equal. apply IH. assumption.

  - (* E_Record *)
    intros fields HFA d1 d2 c1 c2 Hle. simpl. f_equal.
    induction HFA as [| [l' e'] rest He' _ IH].
    + reflexivity.
    + simpl in He'. simpl. rewrite (He' d1 d2 c1 c2 Hle).
      f_equal. exact IH.

  - (* E_Select *) intros e0 l IH d1 d2 c1 c2 Hle. simpl.
    f_equal. apply IH. assumption.

  - (* E_Extend *) intros l e1 e2 IH1 IH2 d1 d2 c1 c2 Hle. simpl.
    f_equal; [apply IH1 | apply IH2]; assumption.

  - (* E_Perform *) intros eff op e0 IH d1 d2 c1 c2 Hle. simpl.
    f_equal. apply IH. assumption.

  - (* E_Handle *) intros h e0 IHh IHe d1 d2 c1 c2 Hle. simpl.
    f_equal; [apply IHh | apply IHe]; assumption.

  - (* E_Resume *) intros e0 IH d1 d2 c1 c2 Hle. simpl.
    f_equal. apply IH. assumption.

  - (* Handler *)
    intros hk e_ret clauses IHret IHclauses d1 d2 c1 c2 Hle. simpl.
    f_equal.
    + apply (IHret d1 d2 (S c1) (S c2)). lia.
    + induction IHclauses as [| cl rest Hcl _ IH].
      * reflexivity.
      * simpl. rewrite (Hcl d1 d2 c1 c2 Hle). f_equal. exact IH.

  - (* OpClause *)
    intros eff op body IH d1 d2 c1 c2 Hle. simpl. f_equal.
    apply (IH d1 d2 (S (S c1)) (S (S c2))). lia.
Qed.

(** ** Substitution commutes with shifting *)

Lemma shift_subst_commute :
  forall e d c j s,
    c <= j ->
    shift_expr d c (subst j s e) =
    subst (j + d) (shift_expr d c s) (shift_expr d c e).
Proof.
  intro e.
  apply (expr_nested_ind
    (fun e => forall d c j s, c <= j ->
       shift_expr d c (subst j s e) =
       subst (j + d) (shift_expr d c s) (shift_expr d c e))
    (fun h => forall d c j s, c <= j ->
       shift_handler d c (subst_handler j s h) =
       subst_handler (j + d) (shift_expr d c s) (shift_handler d c h))
    (fun cl => forall d c j s, c <= j ->
       shift_op_clause d c (subst_op_clause j s cl) =
       subst_op_clause (j + d) (shift_expr d c s) (shift_op_clause d c cl))).

  - (* E_Var *)
    intros v dd cc jj ss Hcj. simpl.
    destruct (jj =? v) eqn:Hjv.
    + (* jj = v *)
      apply Nat.eqb_eq in Hjv. subst v.
      simpl.
      destruct (cc <=? jj) eqn:Hccj.
      * simpl.
        destruct (jj + dd =? jj + dd) eqn:Heq.
        { reflexivity. }
        { apply Nat.eqb_neq in Heq. lia. }
      * apply Nat.leb_gt in Hccj. lia.
    + (* jj <> v *)
      destruct (jj <? v) eqn:Hjv'.
      * (* jj < v *)
        apply Nat.ltb_lt in Hjv'.
        simpl.
        destruct (cc <=? v - 1) eqn:Hcv1.
        { apply Nat.leb_le in Hcv1. simpl.
          destruct (cc <=? v) eqn:Hcv.
          { simpl.
            destruct (jj + dd =? v + dd) eqn:Hjdvd.
            { apply Nat.eqb_eq in Hjdvd. apply Nat.eqb_neq in Hjv. lia. }
            destruct (jj + dd <? v + dd) eqn:Hltjv.
            { f_equal. lia. }
            { apply Nat.ltb_ge in Hltjv. lia. }
          }
          { apply Nat.leb_gt in Hcv. lia. }
        }
        { apply Nat.leb_gt in Hcv1. simpl.
          destruct (cc <=? v) eqn:Hcv.
          { apply Nat.leb_le in Hcv. assert (cc = v) by lia. subst cc. simpl.
            destruct (jj + dd =? v + dd) eqn:Hjdvd.
            { apply Nat.eqb_eq in Hjdvd. apply Nat.eqb_neq in Hjv. lia. }
            destruct (jj + dd <? v + dd) eqn:Hltjv.
            { f_equal. lia. }
            { apply Nat.ltb_ge in Hltjv. lia. }
          }
          { apply Nat.leb_gt in Hcv. lia. }
        }
      * (* jj >= v *)
        apply Nat.ltb_ge in Hjv'. apply Nat.eqb_neq in Hjv.
        assert (jj > v) by lia. simpl.
        destruct (cc <=? v) eqn:Hcv.
        { simpl.
          destruct (jj + dd =? v + dd) eqn:Hjdvd.
          { apply Nat.eqb_eq in Hjdvd. lia. }
          destruct (jj + dd <? v + dd) eqn:Hltjv.
          { apply Nat.ltb_lt in Hltjv. lia. }
          { reflexivity. }
        }
        { simpl.
          destruct (jj + dd =? v) eqn:Hjdv.
          { apply Nat.eqb_eq in Hjdv. lia. }
          destruct (jj + dd <? v) eqn:Hltjdv.
          { apply Nat.ltb_lt in Hltjdv. lia. }
          { reflexivity. }
        }

  - (* E_Const *) intros c dd cc jj ss Hcj. reflexivity.

  - (* E_Lam *)
    intros T body IH dd cc jj ss Hcj. simpl. f_equal.
    rewrite (IH dd (S cc) (S jj) (shift_expr 1 0 ss) ltac:(lia)).
    f_equal. replace (S cc) with (cc + 1) by lia.
    apply shift_shift_commute. lia.

  - (* E_App *)
    intros e1 e2 IH1 IH2 dd cc jj ss Hcj. simpl.
    f_equal; [apply IH1 | apply IH2]; assumption.

  - (* E_Let *)
    intros e1 e2 IH1 IH2 dd cc jj ss Hcj. simpl. f_equal.
    + apply IH1. assumption.
    + rewrite (IH2 dd (S cc) (S jj) (shift_expr 1 0 ss) ltac:(lia)).
      f_equal. replace (S cc) with (cc + 1) by lia.
      apply shift_shift_commute. lia.

  - (* E_Annot *)
    intros e0 T IH dd cc jj ss Hcj. simpl. f_equal. apply IH. assumption.

  - (* E_Record *)
    intros fields HFA dd cc jj ss Hcj. simpl. f_equal.
    induction HFA as [| [l' e'] rest He' _ IH].
    + reflexivity.
    + simpl in He'. simpl. rewrite (He' dd cc jj ss Hcj).
      f_equal. exact IH.

  - (* E_Select *)
    intros e0 l IH dd cc jj ss Hcj. simpl. f_equal. apply IH. assumption.

  - (* E_Extend *)
    intros l e1 e2 IH1 IH2 dd cc jj ss Hcj. simpl.
    f_equal; [apply IH1 | apply IH2]; assumption.

  - (* E_Perform *)
    intros eff op e0 IH dd cc jj ss Hcj. simpl. f_equal. apply IH. assumption.

  - (* E_Handle *)
    intros h e0 IHh IHe dd cc jj ss Hcj. simpl.
    f_equal; [apply IHh | apply IHe]; assumption.

  - (* E_Resume *)
    intros e0 IH dd cc jj ss Hcj. simpl. f_equal. apply IH. assumption.

  - (* Handler *)
    intros hk e_ret clauses IHret IHclauses dd cc jj ss Hcj. simpl.
    f_equal.
    + rewrite (IHret dd (S cc) (S jj) (shift_expr 1 0 ss) ltac:(lia)).
      f_equal. replace (S cc) with (cc + 1) by lia.
      apply shift_shift_commute. lia.
    + induction IHclauses as [| cl rest Hcl _ IH].
      * reflexivity.
      * simpl. rewrite (Hcl dd cc jj ss Hcj). f_equal. exact IH.

  - (* OpClause *)
    intros eff op body IH dd cc jj ss Hcj. simpl. f_equal.
    rewrite (IH dd (S (S cc)) (S (S jj)) (shift_expr 2 0 ss) ltac:(lia)).
    f_equal. replace (S (S cc)) with (cc + 2) by lia.
    apply shift_shift_commute. lia.
Qed.

(** ** Generalized lemma: shifting then substituting cancels *)

Lemma shift_then_subst_general :
  forall e s cutoff,
    subst cutoff s (shift_expr 1 cutoff e) = e.
Proof.
  intro e.
  apply (expr_nested_ind
    (fun e => forall s cutoff, subst cutoff s (shift_expr 1 cutoff e) = e)
    (fun h => forall s cutoff,
       subst_handler cutoff s (shift_handler 1 cutoff h) = h)
    (fun cl => forall s cutoff,
       subst_op_clause cutoff s (shift_op_clause 1 cutoff cl) = cl)).

  - (* E_Var *)
    intros v s cutoff. simpl.
    destruct (cutoff <=? v) eqn:Hle.
    + simpl.
      destruct (cutoff =? v + 1) eqn:Heq.
      * apply Nat.eqb_eq in Heq. apply Nat.leb_le in Hle. lia.
      * destruct (cutoff <? v + 1) eqn:Hlt.
        { f_equal. apply Nat.leb_le in Hle. lia. }
        { apply Nat.ltb_ge in Hlt. apply Nat.leb_le in Hle. lia. }
    + simpl.
      destruct (cutoff =? v) eqn:Heq.
      * apply Nat.eqb_eq in Heq. apply Nat.leb_gt in Hle. lia.
      * destruct (cutoff <? v) eqn:Hlt.
        { apply Nat.ltb_lt in Hlt. apply Nat.leb_gt in Hle. lia. }
        { reflexivity. }

  - (* E_Const *) intros c s cutoff. reflexivity.
  - (* E_Lam *) intros T body IH s cutoff. simpl. f_equal. apply IH.
  - (* E_App *) intros e1 e2 IH1 IH2 s cutoff. simpl.
    f_equal; [apply IH1 | apply IH2].
  - (* E_Let *) intros e1 e2 IH1 IH2 s cutoff. simpl.
    f_equal; [apply IH1 | apply IH2].
  - (* E_Annot *) intros e0 T IH s cutoff. simpl. f_equal. apply IH.

  - (* E_Record *)
    intros fields HFA s cutoff. simpl. f_equal.
    induction HFA as [| [l' e'] rest He' _ IH].
    + reflexivity.
    + simpl in He'. simpl. rewrite (He' s cutoff). f_equal. exact IH.

  - (* E_Select *) intros e0 l IH s cutoff. simpl. f_equal. apply IH.
  - (* E_Extend *) intros l e1 e2 IH1 IH2 s cutoff. simpl.
    f_equal; [apply IH1 | apply IH2].
  - (* E_Perform *) intros eff op e0 IH s cutoff. simpl.
    f_equal. apply IH.
  - (* E_Handle *) intros h e0 IHh IHe s cutoff. simpl.
    f_equal; [apply IHh | apply IHe].
  - (* E_Resume *) intros e0 IH s cutoff. simpl. f_equal. apply IH.

  - (* Handler *)
    intros hk e_ret clauses IHret IHclauses s cutoff. simpl. f_equal.
    + apply IHret.
    + induction IHclauses as [| cl rest Hcl _ IH].
      * reflexivity.
      * simpl. rewrite (Hcl s cutoff). f_equal. exact IH.

  - (* OpClause *)
    intros eff op body IH s cutoff. simpl. f_equal. apply IH.
Qed.

(** ** Identity substitution *)

Lemma subst_shift_cancel :
  forall e v,
    subst 0 v (shift_expr 1 0 e) = e.
Proof.
  intros e v.
  apply shift_then_subst_general.
Qed.
