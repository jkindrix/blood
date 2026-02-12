/* The Computer Language Benchmarks Game
   https://salsa.debian.org/benchmarksgame-team/benchmarksgame/

   contributed by Kevin Carson
   compilation: gcc -O3 -fomit-frame-pointer binarytrees.c -o binarytrees -lm
*/

#include <malloc.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct tn {
    struct tn *left;
    struct tn *right;
} treeNode;

treeNode *NewTreeNode(treeNode *left, treeNode *right) {
    treeNode *new = (treeNode *)malloc(sizeof(treeNode));
    new->left = left;
    new->right = right;
    return new;
}

long ItemCheck(treeNode *tree) {
    if (tree->left == NULL)
        return 1;
    else
        return 1 + ItemCheck(tree->left) + ItemCheck(tree->right);
}

treeNode *BottomUpTree(int depth) {
    if (depth > 0)
        return NewTreeNode(BottomUpTree(depth - 1), BottomUpTree(depth - 1));
    else
        return NewTreeNode(NULL, NULL);
}

void DeleteTree(treeNode *tree) {
    if (tree->left != NULL) {
        DeleteTree(tree->left);
        DeleteTree(tree->right);
    }
    free(tree);
}

int main(int argc, char *argv[]) {
    int n = argc > 1 ? atoi(argv[1]) : 10;
    int minDepth = 4;
    int maxDepth = n;
    if (minDepth + 2 > n)
        maxDepth = minDepth + 2;

    {
        int stretchDepth = maxDepth + 1;
        treeNode *stretchTree = BottomUpTree(stretchDepth);
        printf("stretch tree of depth %d\t check: %ld\n",
               stretchDepth, ItemCheck(stretchTree));
        DeleteTree(stretchTree);
    }

    treeNode *longLivedTree = BottomUpTree(maxDepth);

    for (int depth = minDepth; depth <= maxDepth; depth += 2) {
        int iterations = 1 << (maxDepth - depth + minDepth);
        long check = 0;

        for (int i = 1; i <= iterations; i++) {
            treeNode *tempTree = BottomUpTree(depth);
            check += ItemCheck(tempTree);
            DeleteTree(tempTree);
        }

        printf("%d\t trees of depth %d\t check: %ld\n",
               iterations, depth, check);
    }

    printf("long lived tree of depth %d\t check: %ld\n",
           maxDepth, ItemCheck(longLivedTree));
    DeleteTree(longLivedTree);

    return 0;
}
